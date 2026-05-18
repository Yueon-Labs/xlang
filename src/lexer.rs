use crate::error::{XError, XResult};

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
}

pub struct Lexer {
    chars: Vec<char>,
    i: usize,
    line: usize,
    col: usize,
    tokens: Vec<Token>,
}

impl Lexer {
    pub fn new(source: &str) -> Self {
        Self {
            chars: source.chars().collect(),
            i: 0,
            line: 1,
            col: 1,
            tokens: Vec::new(),
        }
    }

    pub(crate) fn tokenize(mut self) -> XResult<Vec<Token>> {
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
                self.lex_string()?;
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

            return Err(XError::Lex(format!(
                "unexpected character {ch:?} at {}:{}",
                self.line, self.col
            )));
        }

        self.tokens.push(Token {
            kind: TokenKind::Eof,
            text: "<eof>".to_string(),
            line: self.line,
            col: self.col,
        });
        Ok(self.tokens)
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
        if ch == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        ch
    }

    fn push(&mut self, kind: TokenKind, text: String) {
        let width = text.chars().count();
        self.tokens.push(Token {
            kind,
            text,
            line: self.line,
            col: self.col.saturating_sub(width),
        });
    }

    fn lex_ident(&mut self) {
        let start_line = self.line;
        let start_col = self.col;
        let start = self.i;
        while !self.is_eof()
            && (self.peek_char(0).is_ascii_alphanumeric() || self.peek_char(0) == '_')
        {
            self.advance();
        }
        let text: String = self.chars[start..self.i].iter().collect();
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
        });
    }

    fn lex_number(&mut self) {
        let start_line = self.line;
        let start_col = self.col;
        let start = self.i;
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
        let text: String = self.chars[start..self.i].iter().collect();
        self.tokens.push(Token {
            kind,
            text,
            line: start_line,
            col: start_col,
        });
    }

    fn lex_string(&mut self) -> XResult<()> {
        let start_line = self.line;
        let start_col = self.col;
        self.advance(); // opening quote
        let mut value = String::new();
        while !self.is_eof() {
            let ch = self.advance();
            if ch == '"' {
                self.tokens.push(Token {
                    kind: TokenKind::String,
                    text: value,
                    line: start_line,
                    col: start_col,
                });
                return Ok(());
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
        Err(XError::Lex(format!(
            "unterminated string at {start_line}:{start_col}"
        )))
    }

    fn match_multi_symbol(&mut self) -> Option<String> {
        for sym in ["=>", "==", "!=", ">=", "<=", "&&", "||"] {
            let chars: Vec<char> = sym.chars().collect();
            if self.chars.get(self.i..self.i + chars.len()) == Some(chars.as_slice()) {
                for _ in 0..chars.len() {
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
