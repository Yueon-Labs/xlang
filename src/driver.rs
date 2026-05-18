use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::ast::Program;
use crate::codegen::c::CGen;
use crate::error::{XError, XResult};
use crate::lexer::Lexer;
use crate::parser::Parser;

pub fn parse_file(path: &Path) -> XResult<Program> {
    let source = fs::read_to_string(path)?;
    let tokens = Lexer::new(&source).tokenize()?;
    Parser::new(tokens, path.display().to_string()).parse()
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
