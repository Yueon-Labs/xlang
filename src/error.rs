use crate::source::Span;
use serde::Serialize;

// ---------------------------------------------------------------------------
// Fatal error type. Structured diagnostics (lexer / parser / typecheck) flow
// through the `Diagnostic` collector below; this enum is now only for fatal
// conditions: legacy `parse_file` failures (Parse), codegen-unsupported
// (Codegen), and I/O / JSON errors.
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum XError {
    Parse(String),
    Codegen(String),
    Io(std::io::Error),
    Json(serde_json::Error),
}

impl std::fmt::Display for XError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            XError::Parse(msg) => write!(f, "parse error: {msg}"),
            XError::Codegen(msg) => write!(f, "codegen error: {msg}"),
            XError::Io(err) => write!(f, "io error: {err}"),
            XError::Json(err) => write!(f, "json error: {err}"),
        }
    }
}

impl std::error::Error for XError {}

impl From<std::io::Error> for XError {
    fn from(value: std::io::Error) -> Self {
        XError::Io(value)
    }
}

impl From<serde_json::Error> for XError {
    fn from(value: serde_json::Error) -> Self {
        XError::Json(value)
    }
}

pub type XResult<T> = Result<T, XError>;

// ---------------------------------------------------------------------------
// Structured diagnostics — the new foundation for LSP + AI autofix.
// ---------------------------------------------------------------------------

/// Diagnostic severity, mirroring LSP.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
    Information,
    Hint,
}

/// Stable, exhaustive error codes.
///
/// These serialize to fixed `EXXXX` strings — a frozen contract: once a code
/// ships, its serialized form never changes. Autofix engines and AI tooling
/// dispatch on this enum; making it an enum (not a numeric range) means the
/// compiler forces every new code to be handled at `match` sites. See
/// `docs/codes.md` for the human-readable catalogue.
///
/// Numbering: `E1xxx` lexer, `E2xxx` parser, `E3xxx` type checker,
/// `E9xxx` codegen / internal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum ErrorCode {
    // lexer
    #[serde(rename = "E1001")]
    LexUnexpectedChar,
    #[serde(rename = "E1002")]
    LexUnterminatedString,
    // parser
    #[serde(rename = "E2001")]
    ParseUnexpectedToken,
    #[serde(rename = "E2002")]
    ParseExpectedToken,
    #[serde(rename = "E2003")]
    ParseExpectedIdent,
    #[serde(rename = "E2004")]
    ParseUnterminatedBlock,
    #[serde(rename = "E2005")]
    ParseExpectedExpression,
    #[serde(rename = "E2006")]
    ParseUnknownItem,
    // type checker
    #[serde(rename = "E3001")]
    TypeUnknownVar,
    #[serde(rename = "E3002")]
    TypeImmutableAssign,
    #[serde(rename = "E3003")]
    TypeUnknownAssignTarget,
    #[serde(rename = "E3004")]
    TypeAssignmentTarget,
    #[serde(rename = "E3005")]
    TypeMismatch,
    #[serde(rename = "E3006")]
    TypeArgCount,
    #[serde(rename = "E3007")]
    TypeBoolRequired,
    #[serde(rename = "E3008")]
    TypeNumericRequired,
    #[serde(rename = "E3009")]
    TypeOperatorMismatch,
    #[serde(rename = "E3010")]
    TypeForInExpectsSlice,
    #[serde(rename = "E3011")]
    TypeReturnMissingValue,
    // codegen / internal
    #[serde(rename = "E9001")]
    CodegenUnsupported,
    #[serde(rename = "E9002")]
    Internal,
}

/// A single structured diagnostic. `span` is a byte range (converted to
/// line/col by the output layer via [`crate::source::LineIndex`]);
/// `suggestions` (LSP `TextEdit`s) are added in Phase 2 as the autofix surface.
#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: ErrorCode,
    pub message: String,
    pub span: Span,
    pub source: &'static str,
    pub notes: Vec<String>,
}

impl Diagnostic {
    pub fn new(
        code: ErrorCode,
        severity: Severity,
        span: Span,
        message: impl Into<String>,
    ) -> Self {
        Self {
            severity,
            code,
            message: message.into(),
            span,
            source: "xlang",
            notes: Vec::new(),
        }
    }

    pub fn error(code: ErrorCode, span: Span, message: impl Into<String>) -> Self {
        Self::new(code, Severity::Error, span, message)
    }

    #[allow(dead_code)]
    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }
}

/// Accumulating diagnostic sink. Supports reporting many diagnostics from a
/// single compilation (multi-error recovery) instead of bailing at the first.
#[derive(Clone, Debug, Default)]
pub struct Diagnostics {
    pub items: Vec<Diagnostic>,
}

impl Diagnostics {
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    /// Push a diagnostic, dropping exact duplicates (same code + same span).
    /// Prevents one bad variable referenced N times from flooding N copies.
    pub fn push(&mut self, diag: Diagnostic) {
        let dup = self
            .items
            .iter()
            .any(|d| d.code == diag.code && d.span == diag.span);
        if !dup {
            self.items.push(diag);
        }
    }

    pub fn extend(&mut self, other: Diagnostics) {
        for diag in other.items {
            self.push(diag);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn has_errors(&self) -> bool {
        self.items.iter().any(|d| d.severity == Severity::Error)
    }
}

impl From<Diagnostic> for Diagnostics {
    fn from(diag: Diagnostic) -> Self {
        Self { items: vec![diag] }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_code_serializes_to_stable_string() {
        assert_eq!(
            serde_json::to_string(&ErrorCode::TypeUnknownVar).unwrap(),
            "\"E3001\""
        );
        assert_eq!(
            serde_json::to_string(&ErrorCode::LexUnexpectedChar).unwrap(),
            "\"E1001\""
        );
    }

    #[test]
    fn diagnostics_dedup_identical() {
        let mut diags = Diagnostics::new();
        let span = Span::new(0, 5, 10);
        diags.push(Diagnostic::error(ErrorCode::TypeUnknownVar, span, "x"));
        diags.push(Diagnostic::error(ErrorCode::TypeUnknownVar, span, "x"));
        assert_eq!(diags.items.len(), 1);
        // same code, different span → kept
        diags.push(Diagnostic::error(
            ErrorCode::TypeUnknownVar,
            Span::new(0, 20, 25),
            "y",
        ));
        assert_eq!(diags.items.len(), 2);
        assert!(diags.has_errors());
    }
}
