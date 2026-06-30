pub mod source;

pub mod ast;
pub mod cli;
pub mod codegen;
pub mod driver;
pub mod error;
pub mod lsp;
pub mod symbols;
pub mod typecheck;

mod lexer;
mod parser;

pub use error::{XError, XResult};
