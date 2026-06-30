use serde::Serialize;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::ast::Program;
use crate::codegen::c::CGen;
use crate::error::{Diagnostics, ErrorCode, Severity, TextEdit, XError, XResult};
use crate::lexer::Lexer;
use crate::parser::Parser;
use crate::source::LineIndex;
use crate::typecheck::check_program;

const DEFAULT_RUN_SAFE_TIMEOUT_MS: u64 = 2_000;
const DEFAULT_RUN_SAFE_OUTPUT_LIMIT_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug)]
pub struct RunSafeOptions {
    pub timeout_ms: u64,
    pub output_limit_bytes: usize,
}

impl Default for RunSafeOptions {
    fn default() -> Self {
        Self {
            timeout_ms: DEFAULT_RUN_SAFE_TIMEOUT_MS,
            output_limit_bytes: DEFAULT_RUN_SAFE_OUTPUT_LIMIT_BYTES,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSafeResult {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub duration_ms: u64,
    pub timeout_ms: u64,
    pub output_limit_bytes: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug)]
struct CapturedOutput {
    text: String,
    truncated: bool,
}

/// Parse `path` collecting ALL diagnostics (lexer + parser + type checker)
/// rather than bailing at the first. Returns `(program, source, diagnostics)`:
/// `program` is `None` if parsing failed outright; `source` is the file content
/// (for line/col conversion); `diagnostics` may be non-empty even when a program
/// was produced. Only fatal I/O errors propagate via `XResult`.
pub fn parse_collecting(path: &Path) -> XResult<(Option<Program>, String, Diagnostics)> {
    let source = fs::read_to_string(path)?;
    let (program, diags) = parse_source(&source, &path.display().to_string());
    Ok((program, source, diags))
}

/// Like [`parse_collecting`] but takes source text directly (for the LSP server,
/// which holds document text in memory rather than reading a file). Returns the
/// parsed program (if any) plus all accumulated diagnostics.
pub fn parse_source(source: &str, file: &str) -> (Option<Program>, Diagnostics) {
    let mut diags = Diagnostics::new();
    let (tokens, lex_diags) = Lexer::new(source).tokenize();
    diags.extend(lex_diags);
    let program = match Parser::new(tokens, file.to_string()).parse() {
        Ok(program) => Some(program),
        Err(diag) => {
            diags.push(diag);
            None
        }
    };
    if let Some(program) = &program {
        diags.extend(check_program(program));
    }
    (program, diags)
}

/// Legacy single-program entry point for codegen paths: returns the program
/// only when it parsed and type-checked cleanly.
pub fn parse_file(path: &Path) -> XResult<Program> {
    let (program, _source, diags) = parse_collecting(path)?;
    match program {
        Some(program) if !diags.has_errors() => Ok(program),
        _ => Err(XError::Parse(
            diags
                .items
                .iter()
                .map(|d| d.message.clone())
                .collect::<Vec<_>>()
                .join("; "),
        )),
    }
}

/// Machine-readable diagnostic for JSON output. Mirrors an LSP `Diagnostic`
/// (severity + code + message + range); the range carries both 1-based
/// line/col (resolved from bytes) and the raw byte span.
#[derive(Debug, Serialize)]
pub struct SerializableDiagnostic {
    pub severity: Severity,
    pub code: ErrorCode,
    pub message: String,
    pub file: String,
    pub range: SerializableRange,
    pub suggestions: Vec<SerializableTextEdit>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SerializableRange {
    pub line: usize,
    pub col: usize,
    pub end_line: usize,
    pub end_col: usize,
    pub start: u32,
    pub end: u32,
}

/// A machine-applicable fix in JSON output: replace `range` with `newText`.
#[derive(Debug, Serialize)]
pub struct SerializableTextEdit {
    pub range: SerializableRange,
    #[serde(rename = "newText")]
    pub new_text: String,
}

/// Convert accumulated diagnostics to serializable form, resolving byte spans
/// to 1-based line/col via `LineIndex`.
pub fn diagnostics_to_serializable(
    diags: &Diagnostics,
    source: &str,
    file: &str,
) -> Vec<SerializableDiagnostic> {
    let index = LineIndex::new(source);
    diags
        .items
        .iter()
        .map(|d| {
            let (line, col) = index.line_col(d.span.start);
            let end_offset = d.span.end.saturating_sub(1).max(d.span.start);
            let (end_line, end_col) = index.line_col(end_offset);
            let suggestions = d
                .suggestions
                .iter()
                .map(|s| {
                    let (sl, sc) = index.line_col(s.range.start);
                    let seo = s.range.end.saturating_sub(1).max(s.range.start);
                    let (sel, sec) = index.line_col(seo);
                    SerializableTextEdit {
                        range: SerializableRange {
                            line: sl,
                            col: sc,
                            end_line: sel,
                            end_col: sec,
                            start: s.range.start,
                            end: s.range.end,
                        },
                        new_text: s.new_text.clone(),
                    }
                })
                .collect();
            SerializableDiagnostic {
                severity: d.severity,
                code: d.code,
                message: d.message.clone(),
                file: file.to_string(),
                range: SerializableRange {
                    line,
                    col,
                    end_line,
                    end_col,
                    start: d.span.start,
                    end: d.span.end,
                },
                suggestions,
            }
        })
        .collect()
}

/// Apply all `suggestions` from `diags` to `source`, returning the fixed text.
/// Edits are applied right-to-left (sorted by start descending) so earlier byte
/// offsets stay valid. Clamped to be safe against out-of-range edits.
pub fn apply_suggestions(source: &str, diags: &Diagnostics) -> String {
    let mut edits: Vec<&TextEdit> = diags
        .items
        .iter()
        .flat_map(|d| d.suggestions.iter())
        .collect();
    edits.sort_by_key(|e| std::cmp::Reverse(e.range.start));
    let mut out: Vec<u8> = source.as_bytes().to_vec();
    for edit in edits {
        let s = edit.range.start as usize;
        let e = edit.range.end as usize;
        if s <= out.len() && e <= out.len() && s <= e {
            out.splice(s..e, edit.new_text.as_bytes().iter().copied());
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Count the total machine-applicable suggestions across all diagnostics.
pub fn suggestion_count(diags: &Diagnostics) -> usize {
    diags.items.iter().map(|d| d.suggestions.len()).sum()
}

/// gcc-style lines: `file:line:col: severity[code]: message`.
pub fn diagnostics_to_gcc(diags: &Diagnostics, source: &str, file: &str) -> Vec<String> {
    let index = LineIndex::new(source);
    diags
        .items
        .iter()
        .map(|d| {
            let (line, col) = index.line_col(d.span.start);
            let sev = match d.severity {
                Severity::Error => "error",
                Severity::Warning => "warning",
                Severity::Information => "note",
                Severity::Hint => "hint",
            };
            let code = serde_json::to_string(&d.code)
                .unwrap_or_else(|_| "\"E9002\"".into())
                .trim_matches('"')
                .to_string();
            format!("{file}:{line}:{col}: {sev}[{code}]: {}", d.message)
        })
        .collect()
}

pub fn write_c(source: &Path, output: Option<PathBuf>) -> XResult<PathBuf> {
    let program = parse_file(source)?;
    let c_code = CGen::new().generate(&program)?;
    let output = output.unwrap_or_else(|| {
        let stem = source.file_stem().unwrap_or_default().to_string_lossy();
        PathBuf::from("build").join(format!("{stem}.c"))
    });
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&output, c_code)?;
    Ok(output)
}

pub fn build_exe(source: &Path, output: Option<PathBuf>) -> XResult<PathBuf> {
    fs::create_dir_all("build")?;
    let stem = source.file_stem().unwrap_or_default().to_string_lossy();
    let c_path = PathBuf::from("build").join(format!("{stem}.c"));
    write_c(source, Some(c_path.clone()))?;
    let exe = output.unwrap_or_else(|| PathBuf::from("build").join(stem.as_ref()));
    let out = Command::new("cc")
        .arg(&c_path)
        .arg("-o")
        .arg(&exe)
        .output()?;
    if !out.status.success() {
        eprint!("{}", String::from_utf8_lossy(&out.stderr));
        return Err(XError::Codegen(format!(
            "C compiler failed with exit code {:?}",
            out.status.code()
        )));
    }
    Ok(exe)
}

pub fn run_safe(source: &Path, options: RunSafeOptions) -> XResult<RunSafeResult> {
    let started = Instant::now();
    let work_dir = safe_work_dir()?;
    fs::create_dir_all(&work_dir)?;

    let result = run_safe_in_dir(source, &work_dir, &options, started);
    let _ = fs::remove_dir_all(&work_dir);
    result
}

fn run_safe_in_dir(
    source: &Path,
    work_dir: &Path,
    options: &RunSafeOptions,
    started: Instant,
) -> XResult<RunSafeResult> {
    let c_path = work_dir.join("program.c");
    let exe_path = work_dir.join(exe_name("program"));

    if let Err(err) = write_c(source, Some(c_path.clone())) {
        return Ok(result_from_xerror(err, started, options));
    }

    let compile_out = match Command::new("cc")
        .arg(&c_path)
        .arg("-o")
        .arg(&exe_path)
        .output()
    {
        Ok(out) => out,
        Err(err) => {
            return Ok(RunSafeResult::error(
                "compile_error",
                format!("failed to start C compiler: {err}"),
                started,
                options,
            ));
        }
    };

    if !compile_out.status.success() {
        return Ok(RunSafeResult {
            status: "compile_error".to_string(),
            exit_code: compile_out.status.code(),
            stdout: capture_command_output(&compile_out.stdout, options.output_limit_bytes).text,
            stderr: capture_command_output(&compile_out.stderr, options.output_limit_bytes).text,
            stdout_truncated: compile_out.stdout.len() > options.output_limit_bytes,
            stderr_truncated: compile_out.stderr.len() > options.output_limit_bytes,
            duration_ms: elapsed_ms(started),
            timeout_ms: options.timeout_ms,
            output_limit_bytes: options.output_limit_bytes,
            error: Some(format!(
                "C compiler failed with exit code {:?}",
                compile_out.status.code()
            )),
        });
    }

    run_executable_safe(&exe_path, options, started)
}

fn run_executable_safe(
    exe_path: &Path,
    options: &RunSafeOptions,
    started: Instant,
) -> XResult<RunSafeResult> {
    let exe = fs::canonicalize(exe_path)?;
    let mut child = Command::new(exe)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let stdout_handle = child.stdout.take().map(|stdout| {
        let limit = options.output_limit_bytes;
        thread::spawn(move || read_limited(stdout, limit))
    });
    let stderr_handle = child.stderr.take().map(|stderr| {
        let limit = options.output_limit_bytes;
        thread::spawn(move || read_limited(stderr, limit))
    });

    let timeout = Duration::from_millis(options.timeout_ms);
    // The execution deadline starts when the program launches, NOT from
    // `started` (run_safe entry) — otherwise slow C compilation on loaded
    // CI runners eats the whole timeout budget before the program even runs.
    let exec_started = Instant::now();
    let mut timed_out = false;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if exec_started.elapsed() >= timeout {
            timed_out = true;
            child.kill()?;
            break child.wait()?;
        }
        thread::sleep(Duration::from_millis(10));
    };

    let stdout = join_capture(stdout_handle)?;
    let stderr = join_capture(stderr_handle)?;

    Ok(RunSafeResult {
        status: if timed_out { "timeout" } else { "ok" }.to_string(),
        exit_code: if timed_out { None } else { status.code() },
        stdout: stdout.text,
        stderr: stderr.text,
        stdout_truncated: stdout.truncated,
        stderr_truncated: stderr.truncated,
        duration_ms: elapsed_ms(started),
        timeout_ms: options.timeout_ms,
        output_limit_bytes: options.output_limit_bytes,
        error: timed_out.then(|| format!("program timed out after {} ms", options.timeout_ms)),
    })
}

impl RunSafeResult {
    fn error(
        status: impl Into<String>,
        error: impl Into<String>,
        started: Instant,
        options: &RunSafeOptions,
    ) -> Self {
        Self {
            status: status.into(),
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            stdout_truncated: false,
            stderr_truncated: false,
            duration_ms: elapsed_ms(started),
            timeout_ms: options.timeout_ms,
            output_limit_bytes: options.output_limit_bytes,
            error: Some(error.into()),
        }
    }
}

fn result_from_xerror(err: XError, started: Instant, options: &RunSafeOptions) -> RunSafeResult {
    let status = match &err {
        XError::Parse(_) => "check_error",
        XError::Codegen(_) => "codegen_error",
        XError::Io(_) | XError::Json(_) => "error",
    };
    RunSafeResult::error(status, err.to_string(), started, options)
}

fn read_limited<R: Read>(mut reader: R, limit: usize) -> io::Result<CapturedOutput> {
    let mut bytes = Vec::new();
    let mut truncated = false;
    let mut chunk = [0_u8; 8192];

    loop {
        let n = reader.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        let remaining = limit.saturating_sub(bytes.len());
        if remaining > 0 {
            let take = remaining.min(n);
            bytes.extend_from_slice(&chunk[..take]);
        }
        if n > remaining {
            truncated = true;
        }
    }

    Ok(CapturedOutput {
        text: String::from_utf8_lossy(&bytes).to_string(),
        truncated,
    })
}

fn capture_command_output(bytes: &[u8], limit: usize) -> CapturedOutput {
    let take = limit.min(bytes.len());
    CapturedOutput {
        text: String::from_utf8_lossy(&bytes[..take]).to_string(),
        truncated: bytes.len() > limit,
    }
}

fn join_capture(
    handle: Option<thread::JoinHandle<io::Result<CapturedOutput>>>,
) -> io::Result<CapturedOutput> {
    let Some(handle) = handle else {
        return Ok(CapturedOutput {
            text: String::new(),
            truncated: false,
        });
    };
    handle
        .join()
        .map_err(|_| io::Error::other("output capture thread panicked"))?
}

fn safe_work_dir() -> XResult<PathBuf> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| XError::Io(io::Error::other(err)))?
        .as_nanos();
    Ok(PathBuf::from("build")
        .join("safe")
        .join(format!("{}-{stamp}", std::process::id())))
}

fn exe_name(stem: &str) -> String {
    if cfg!(windows) {
        format!("{stem}.exe")
    } else {
        stem.to_string()
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis().try_into().unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::{RunSafeOptions, read_limited, run_safe};
    use std::fs;
    use std::io::Cursor;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn read_limited_marks_truncated_output() {
        let captured = read_limited(Cursor::new(b"abcdef"), 3).expect("capture output");

        assert_eq!(captured.text, "abc");
        assert!(captured.truncated);
    }

    #[test]
    fn run_safe_reports_successful_execution() {
        let source = write_temp_source(
            "run-ok",
            r#"
module main

fn main(): i32 {
    return 1
}
"#,
        );

        let result = run_safe(
            &source,
            RunSafeOptions {
                timeout_ms: 2_000,
                output_limit_bytes: 4096,
            },
        )
        .expect("run safe");
        let _ = fs::remove_file(source);

        assert_eq!(result.status, "ok");
        assert_eq!(result.exit_code, Some(1));
    }

    #[test]
    fn run_safe_times_out_nonterminating_program() {
        let source = write_temp_source(
            "run-timeout",
            r#"
module main

fn main(): i32 {
    while true {
    }
    return 0
}
"#,
        );

        let result = run_safe(
            &source,
            RunSafeOptions {
                timeout_ms: 100,
                output_limit_bytes: 4096,
            },
        )
        .expect("run safe");
        let _ = fs::remove_file(source);

        assert_eq!(result.status, "timeout");
        assert_eq!(result.exit_code, None);
    }

    fn write_temp_source(prefix: &str, source: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("xlang-run-safe-tests");
        fs::create_dir_all(&dir).expect("create temp source dir");
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let path = dir.join(format!("{prefix}-{}-{stamp}.x", std::process::id()));
        fs::write(&path, source).expect("write temp source");
        path
    }
}
