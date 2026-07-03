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

/// A lexed token. Position is byte offsets only (`start` inclusive, `end`
/// exclusive); line/column are derived on demand via `LineIndex`, so bytes are
/// the single source of truth.
#[derive(Clone, Debug)]
pub(crate) struct Token {
    pub(crate) kind: TokenKind,
    pub(crate) text: String,
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

            if "{}()[]:,.<>&|^~+-*/%=!".contains(ch) {
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
        ch
    }

    /// Span covering the next `len` bytes starting at the current position.
    fn span_here(&self, len: usize) -> Span {
        let start = self.byte_offset as u32;
        Span::new(self.file_id, start, start + len as u32)
    }

    fn push(&mut self, kind: TokenKind, text: String) {
        let end = self.byte_offset as u32;
        let start = end.saturating_sub(text.len() as u32);
        self.tokens.push(Token {
            kind,
            text,
            start,
            end,
        });
    }

    fn lex_ident(&mut self) {
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
            start: start_byte as u32,
            end: end_byte as u32,
        });
    }

    fn lex_number(&mut self) {
        let start_byte = self.byte_offset;
        // Based integer literal: 0x (hex), 0b (binary), 0o (octal). These are
        // parsed to their decimal value so codegen can emit a plain decimal
        // literal (uniform, C-valid on every compiler — `0b..` is C23-only and
        // `0o..` isn't C at all). A leading 0 followed by a non-base char is a
        // normal decimal (e.g. `0`, `08`).
        if self.peek_char(0) == '0' && !self.is_eof() {
            match self.peek_char(1) {
                'x' | 'X' => {
                    self.advance();
                    self.advance();
                    self.lex_based_int(16, start_byte as u32);
                    return;
                }
                'b' | 'B' => {
                    self.advance();
                    self.advance();
                    self.lex_based_int(2, start_byte as u32);
                    return;
                }
                'o' | 'O' => {
                    self.advance();
                    self.advance();
                    self.lex_based_int(8, start_byte as u32);
                    return;
                }
                _ => {}
            }
        }
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
            start: start_byte as u32,
            end: end_byte as u32,
        });
    }

    /// Lex the digit run of a based integer (base 2/8/16) and emit an Int token
    /// whose text is the decimal value. Underscore separators (`0xFF_FF`) are
    /// allowed and stripped before parsing.
    fn lex_based_int(&mut self, base: u32, start_byte: u32) {
        let digit_start = self.i;
        while !self.is_eof() && self.is_digit_in_base(self.peek_char(0), base) {
            self.advance();
        }
        let raw: String = self.chars[digit_start..self.i]
            .iter()
            .filter(|c| **c != '_')
            .collect();
        // from_str_radix rejects empty / out-of-range; fall back to 0 so a
        // malformed literal like `0x` degrades to 0 rather than panicking.
        let value = u64::from_str_radix(&raw, base).unwrap_or(0);
        let end_byte = self.byte_offset;
        self.tokens.push(Token {
            kind: TokenKind::Int,
            text: value.to_string(),
            start: start_byte,
            end: end_byte as u32,
        });
    }

    fn is_digit_in_base(&self, c: char, base: u32) -> bool {
        match base {
            16 => c.is_ascii_hexdigit() || c == '_',
            8 => ('0'..='7').contains(&c) || c == '_',
            2 => c == '0' || c == '1' || c == '_',
            _ => c.is_ascii_digit(),
        }
    }

    fn lex_string(&mut self) {
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
                    'e' => '\x1b',
                    other => other,
                });
            } else {
                value.push(ch);
            }
        }
        // Unterminated string: report and continue (no token emitted).
        let span = Span::new(self.file_id, start_byte as u32, self.byte_offset as u32);
        self.diags.push(Diagnostic::error(
            ErrorCode::LexUnterminatedString,
            span,
            "unterminated string literal",
        ));
    }

    fn match_multi_symbol(&mut self) -> Option<String> {
        for sym in [
            "=>", "==", "!=", ">=", "<=", "&&", "||", "+=", "-=", "*=", "/=", "%=", "<<", ">>",
            "..=", "..",
        ] {
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
            | "impl"
            | "enum"
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

#[cfg(test)]
mod tests {
    use super::*;

    fn lex(src: &str) -> (Vec<Token>, Vec<String>) {
        let (toks, diags) = Lexer::new(src).tokenize();
        let msgs = diags.items.iter().map(|d| d.message.clone()).collect();
        (toks, msgs)
    }

    #[test]
    fn lexes_numbers_idents_keywords() {
        let (toks, diags) = lex("fn 42 x");
        assert!(diags.is_empty(), "unexpected diags: {diags:?}");
        assert_eq!(toks[0].kind, TokenKind::Keyword);
        assert_eq!(toks[0].text, "fn");
        assert_eq!(toks[1].kind, TokenKind::Int);
        assert_eq!(toks[1].text, "42");
        assert_eq!(toks[2].kind, TokenKind::Ident);
        assert_eq!(toks[2].text, "x");
    }

    #[test]
    fn recognizes_multi_char_symbols() {
        let (toks, _) = lex("=> == != >= <= && || += -=");
        let texts: Vec<&str> = toks.iter().map(|t| t.text.as_str()).collect();
        assert_eq!(
            texts,
            vec![
                "=>", "==", "!=", ">=", "<=", "&&", "||", "+=", "-=", "<eof>"
            ]
        );
    }

    #[test]
    fn lexes_range_operator() {
        // `0..10` must NOT be swallowed as a float: the number lexer only
        // extends past '.' when the next char is a digit, so `0` is an int and
        // `..` is its own symbol. `..=` is the inclusive form (longest match).
        let (toks, _) = lex("0..10");
        let texts: Vec<&str> = toks.iter().map(|t| t.text.as_str()).collect();
        assert_eq!(texts, vec!["0", "..", "10", "<eof>"]);
        let (toks3, _) = lex("0..=10");
        let texts3: Vec<&str> = toks3.iter().map(|t| t.text.as_str()).collect();
        assert_eq!(texts3, vec!["0", "..=", "10", "<eof>"]);
        // A single dot (field access) is unaffected by adding `..`/`..=`.
        let (toks2, _) = lex("a.b");
        let texts2: Vec<&str> = toks2.iter().map(|t| t.text.as_str()).collect();
        assert_eq!(texts2, vec!["a", ".", "b", "<eof>"]);
    }

    #[test]
    fn lexes_based_int_literals() {
        // 0x/0b/0o prefixes parse to their decimal value (so codegen emits a
        // plain decimal literal, C-valid on every compiler).
        let cases = [
            ("0xFF", "255"),
            ("0x1A", "26"),
            ("0b1010", "10"),
            ("0b1111", "15"),
            ("0o17", "15"),
            ("0o777", "511"),
            ("0xFF_FF", "65535"), // underscore separator allowed
            ("0", "0"),           // plain zero stays decimal
            ("255", "255"),       // plain decimal unaffected
        ];
        for (src, want) in cases {
            let (toks, _) = lex(src);
            assert_eq!(toks[0].kind, TokenKind::Int, "{src}: kind");
            assert_eq!(toks[0].text, want, "{src}: value");
        }
    }

    #[test]
    fn byte_offsets_ascii() {
        let (toks, _) = lex("ab 12");
        assert_eq!((toks[0].start, toks[0].end), (0, 2), "ab span");
        assert_eq!((toks[1].start, toks[1].end), (3, 5), "12 span");
    }

    #[test]
    fn byte_offsets_count_multibyte_as_bytes() {
        // `"中"` = `"`(1) + 中(3 bytes) + `"`(1) = bytes 0..5. A char-indexed
        // lexer (the bug) would report 0..3 — this guards against that.
        let (toks, _) = lex("\"中\"");
        let s = toks
            .iter()
            .find(|t| t.kind == TokenKind::String)
            .expect("a string token");
        assert_eq!((s.start, s.end), (0, 5), "中 must count as 3 bytes");
    }

    #[test]
    fn recovers_from_multiple_bad_chars() {
        let (toks, diags) = lex("@ #");
        assert_eq!(diags.len(), 2, "expected 2 lex errors, got {diags:?}");
        assert!(toks.iter().any(|t| t.kind == TokenKind::Eof));
    }

    #[test]
    fn reports_unterminated_string() {
        let (_toks, diags) = lex("\"abc");
        assert_eq!(diags.len(), 1, "got {diags:?}");
        assert!(diags[0].contains("unterminated string"));
    }

    #[test]
    fn string_escapes_are_decoded() {
        let (toks, _) = lex("\"a\\nb\"");
        let s = toks
            .iter()
            .find(|t| t.kind == TokenKind::String)
            .expect("string token");
        assert_eq!(s.text, "a\nb", "escape decoded into the token text");
    }
}
