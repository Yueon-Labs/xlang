use crate::error::{Diagnostic, Diagnostics, ErrorCode};
use crate::source::Span;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TokenKind {
    Keyword,
    Ident,
    Int,
    Float,
    String,
    Symbol,
    Eof,
}

#[derive(Clone, Debug)]
pub(crate) struct Token {
    pub(crate) kind: TokenKind,
    pub(crate) text: String,
    pub(crate) line: usize,
    pub(crate) col: usize,
    /// Byte offset of the first character of the token (inclusive).
    pub(crate) start: u32,
    /// Byte offset just past the last character of the token (exclusive).
    pub(crate) end: u32,
}

pub struct Lexer {
    chars: Vec<char>,
    i: usize,
    /// Byte offset in the original source (advances by `char.len_utf8()` per
    /// consumed char, so it stays correct for non-ASCII — unlike the char index `i`).
    byte_offset: usize,
    line: usize,
    col: usize,
    tokens: Vec<Token>,
    diags: Diagnostics,
    file_id: u32,
}

impl Lexer {
    pub fn new(source: &str) -> Self {
        Self {
            chars: source.chars().collect(),
            i: 0,
            byte_offset: 0,
            line: 1,
            col: 1,
            tokens: Vec::new(),
            diags: Diagnostics::new(),
            file_id: 0,
        }
    }

    /// Tokenize, accumulating diagnostics instead of bailing at the first bad
    /// character. Returns `(tokens, diagnostics)`; an unexpected character is
    /// reported and skipped so later errors are still surfaced (multi-error
    /// recovery). The returned token stream is always produced (up to the point
    /// the lexer reached), even when diagnostics are present.
    pub(crate) fn tokenize(mut self) -> (Vec<Token>, Diagnostics) {
        while !self.is_eof() {
            let ch = self.peek_char(0);

            if ch.is_whitespace() {
                self.advance();
                continue;
            }

            if ch == '/' && self.peek_char(1) == '/' {
                while !self.is_eof() && self.peek_char(0) != '\n' {
                    self.advance();
                }
                continue;
            }

            if ch.is_ascii_alphabetic() || ch == '_' {
                self.lex_ident();
                continue;
            }

            if ch.is_ascii_digit() {
                self.lex_number();
                continue;
            }

            if ch == '"' {
                self.lex_string();
                continue;
            }

            if let Some(sym) = self.match_multi_symbol() {
                self.push(TokenKind::Symbol, sym);
                continue;
            }

            if "{}()[]:,.<>+-*/%=!".contains(ch) {
                let text = ch.to_string();
                self.advance();
                self.push(TokenKind::Symbol, text);
                continue;
            }

            // Unexpected character: report it and skip, then keep going so we
            // can surface more than one lex error per file.
            let span = self.span_here(ch.len_utf8());
            self.diags.push(Diagnostic::error(
                ErrorCode::LexUnexpectedChar,
                span,
                format!("unexpected character {ch:?}"),
            ));
            self.advance();
        }

        let end = self.byte_offset as u32;
        self.tokens.push(Token {
            kind: TokenKind::Eof,
            text: "<eof>".to_string(),
            line: self.line,
            col: self.col,
            start: end,
            end,
        });
        (self.tokens, self.diags)
    }

    fn is_eof(&self) -> bool {
        self.i >= self.chars.len()
    }

    fn peek_char(&self, offset: usize) -> char {
        self.chars.get(self.i + offset).copied().unwrap_or('\0')
    }

    fn advance(&mut self) -> char {
        let ch = self.chars[self.i];
        self.i += 1;
        self.byte_offset += ch.len_utf8();
        if ch == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        ch
    }

    /// Span covering the next `len` bytes starting at the current position.
    fn span_here(&self, len: usize) -> Span {
        let start = self.byte_offset as u32;
        Span::new(self.file_id, start, start + len as u32)
    }

    fn push(&mut self, kind: TokenKind, text: String) {
        let width_chars = text.chars().count();
        let end = self.byte_offset as u32;
        let start = end.saturating_sub(text.len() as u32);
        self.tokens.push(Token {
            kind,
            text,
            line: self.line,
            col: self.col.saturating_sub(width_chars),
            start,
            end,
        });
    }

    fn lex_ident(&mut self) {
        let start_line = self.line;
        let start_col = self.col;
        let start_byte = self.byte_offset;
        let start_idx = self.i;
        while !self.is_eof()
            && (self.peek_char(0).is_ascii_alphanumeric() || self.peek_char(0) == '_')
        {
            self.advance();
        }
        let text: String = self.chars[start_idx..self.i].iter().collect();
        let end_byte = self.byte_offset;
        let kind = if is_keyword(&text) {
            TokenKind::Keyword
        } else {
            TokenKind::Ident
        };
        self.tokens.push(Token {
            kind,
            text,
            line: start_line,
            col: start_col,
            start: start_byte as u32,
            end: end_byte as u32,
        });
    }

    fn lex_number(&mut self) {
        let start_line = self.line;
        let start_col = self.col;
        let start_byte = self.byte_offset;
        let start_idx = self.i;
        while !self.is_eof() && self.peek_char(0).is_ascii_digit() {
            self.advance();
        }
        let mut kind = TokenKind::Int;
        if !self.is_eof() && self.peek_char(0) == '.' && self.peek_char(1).is_ascii_digit() {
            kind = TokenKind::Float;
            self.advance();
            while !self.is_eof() && self.peek_char(0).is_ascii_digit() {
                self.advance();
            }
        }
        let text: String = self.chars[start_idx..self.i].iter().collect();
        let end_byte = self.byte_offset;
        self.tokens.push(Token {
            kind,
            text,
            line: start_line,
            col: start_col,
            start: start_byte as u32,
            end: end_byte as u32,
        });
    }

    fn lex_string(&mut self) {
        let start_line = self.line;
        let start_col = self.col;
        let start_byte = self.byte_offset;
        self.advance(); // opening quote
        let mut value = String::new();
        while !self.is_eof() {
            let ch = self.advance();
            if ch == '"' {
                let end_byte = self.byte_offset;
                self.tokens.push(Token {
                    kind: TokenKind::String,
                    text: value,
                    line: start_line,
                    col: start_col,
                    start: start_byte as u32,
                    end: end_byte as u32,
                });
                return;
            }
            if ch == '\\' {
                if self.is_eof() {
                    break;
                }
                let esc = self.advance();
                value.push(match esc {
                    'n' => '\n',
                    't' => '\t',
                    'r' => '\r',
                    '"' => '"',
                    '\\' => '\\',
                    other => other,
                });
            } else {
                value.push(ch);
            }
        }
        // Unterminated string: report and continue (no token emitted).
        let span = Span::new(
            self.file_id,
            start_byte as u32,
            self.byte_offset as u32,
        );
        self.diags.push(Diagnostic::error(
            ErrorCode::LexUnterminatedString,
            span,
            "unterminated string literal",
        ));
    }

    fn match_multi_symbol(&mut self) -> Option<String> {
        for sym in ["=>", "==", "!=", ">=", "<=", "&&", "||"] {
            let sym_chars: Vec<char> = sym.chars().collect();
            if self.chars.get(self.i..self.i + sym_chars.len()) == Some(sym_chars.as_slice()) {
                for _ in 0..sym_chars.len() {
                    self.advance();
                }
                return Some(sym.to_string());
            }
        }
        None
    }
}

fn is_keyword(text: &str) -> bool {
    matches!(
        text,
        "module"
            | "import"
            | "struct"
            | "type"
            | "fn"
            | "let"
            | "mut"
            | "if"
            | "else"
            | "for"
            | "in"
            | "while"
            | "match"
            | "return"
            | "break"
            | "continue"
            | "true"
            | "false"
    )
}
