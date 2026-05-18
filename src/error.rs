#[derive(Debug)]
pub enum XError {
    Lex(String),
    Parse(String),
    Codegen(String),
    Io(std::io::Error),
    Json(serde_json::Error),
}

impl std::fmt::Display for XError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            XError::Lex(msg) => write!(f, "lexer error: {msg}"),
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
