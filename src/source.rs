//! Source positions and the span-aware AST wrapper.
//!
//! `Span` stores byte offsets only (`start` inclusive, `end` exclusive).
//! Line/column are derived on demand via [`LineIndex`], so bytes are the single
//! source of truth and there is no stale line/col to keep in sync. `file_id`
//! indexes a future multi-file registry; Phase 1 hard-codes `file_id = 0`.

use serde::{Serialize, Serializer};

/// Byte range in a source file. `start` is inclusive, `end` exclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct Span {
    pub file_id: u32,
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub const fn new(file_id: u32, start: u32, end: u32) -> Self {
        Self {
            file_id,
            start,
            end,
        }
    }

    /// Zero-length span at offset 0 — fallback when no real position is known.
    pub const fn unknown(file_id: u32) -> Self {
        Self {
            file_id,
            start: 0,
            end: 0,
        }
    }

    /// Smallest span covering both `self` and `other` (assumes same file).
    pub fn merge(self, other: Span) -> Span {
        Span {
            file_id: self.file_id,
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }
}

/// A syntax node paired with the span it occupied in source.
///
/// Serializes as the inner `node` only (span dropped), so wrapping existing
/// AST nodes does not change the `ast` subcommand's JSON output. We deliberately
/// hand-roll [`Serialize`] instead of using `#[serde(flatten)]`: serde's
/// `flatten` is incompatible with the `#[serde(tag = "kind")]` internally-tagged
/// enums used by `Item`/`Stmt`/`Expr` (it loses the tag), whereas delegating
/// directly to `T::serialize` works for any `T: Serialize`.
#[derive(Clone, Debug)]
pub struct Spanned<T> {
    pub node: T,
    pub span: Span,
}

impl<T> Spanned<T> {
    /// Create a spanned node. Asserts the invariant `start <= end`.
    pub fn new(node: T, span: Span) -> Self {
        debug_assert!(
            span.start <= span.end,
            "span start {} > end {}",
            span.start,
            span.end
        );
        Self { node, span }
    }

    /// Apply a function to the inner node, keeping the span.
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Spanned<U> {
        Spanned {
            node: f(self.node),
            span: self.span,
        }
    }

    pub fn as_ref(&self) -> Spanned<&T> {
        Spanned {
            node: &self.node,
            span: self.span,
        }
    }
}

impl<T: Serialize> Serialize for Spanned<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.node.serialize(serializer)
    }
}

/// Index of line-start byte offsets, for converting a byte offset to a 1-based
/// `(line, col)`. Built once per source string.
#[derive(Clone, Debug)]
pub struct LineIndex {
    /// Byte offset of the first character of each line (line N starts at
    /// `line_starts[N - 1]`). Line 1 always starts at offset 0.
    line_starts: Vec<u32>,
}

impl LineIndex {
    pub fn new(source: &str) -> Self {
        let mut line_starts = vec![0u32];
        for (byte_off, ch) in source.char_indices() {
            if ch == '\n' {
                // The next line begins immediately after the newline.
                line_starts.push((byte_off + 1) as u32);
            }
        }
        LineIndex { line_starts }
    }

    /// 1-based `(line, column)` for `byte_offset`. Column is 1-based and counts
    /// bytes from the line start (so a multi-byte char advances the column by
    /// its byte width — consistent with byte spans). Clamps to the last line if
    /// the offset is at/past EOF.
    pub fn line_col(&self, byte_offset: u32) -> (usize, usize) {
        let off = byte_offset as usize;
        let line_idx = match self.line_starts.binary_search(&byte_offset) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        };
        let line_start = self.line_starts[line_idx] as usize;
        let line = line_idx + 1;
        let col = off.saturating_sub(line_start) + 1;
        (line, col)
    }

    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    /// Byte offset of 1-based `(line, col)`. `line` is clamped to the last
    /// line; the result is clamped to the source length. Used by the LSP to
    /// map a cursor position to a byte offset (e.g. for references).
    pub fn byte_offset(&self, line: u32, col: u32) -> u32 {
        let li = (line as usize)
            .saturating_sub(1)
            .min(self.line_starts.len().saturating_sub(1));
        self.line_starts[li] + col.saturating_sub(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_col_ascii() {
        // bytes: a=0 b=1 \n=2 c=3 d=4 \n=5 e=6 f=7
        let idx = LineIndex::new("ab\ncd\nef");
        assert_eq!(idx.line_col(0), (1, 1)); // 'a'
        assert_eq!(idx.line_col(1), (1, 2)); // 'b'
        assert_eq!(idx.line_col(3), (2, 1)); // 'c' (after \n at 2)
        assert_eq!(idx.line_col(6), (3, 1)); // 'e' (after \n at 5)
        assert_eq!(idx.line_count(), 3);
    }

    #[test]
    fn line_col_multibyte() {
        // '中' = bytes 0..3, '文' = bytes 3..6, \n = 6, 'x' = 7
        let idx = LineIndex::new("中文\nx");
        assert_eq!(idx.line_col(0), (1, 1)); // 中 start
        assert_eq!(idx.line_col(3), (1, 4)); // 文 start (byte offset 3 → col 4)
        assert_eq!(idx.line_col(7), (2, 1)); // 'x'
    }

    #[test]
    fn spanned_serializes_as_inner_node() {
        let spanned: Spanned<i32> = Spanned::new(42, Span::new(0, 10, 12));
        assert_eq!(serde_json::to_string(&spanned).unwrap(), "42");
    }

    #[test]
    fn span_merge_covers_both() {
        let a = Span::new(0, 5, 10);
        let b = Span::new(0, 8, 20);
        assert_eq!(a.merge(b), Span::new(0, 5, 20));
    }

    #[test]
    fn spanned_through_internally_tagged_enum_serializes_with_tag() {
        // Regression guard for the flatten-vs-internally-tagged decision:
        // a Spanned<Example> must serialize exactly like Example (tag included).
        #[derive(Serialize)]
        #[serde(tag = "kind")]
        enum Example {
            Leaf { value: i32 },
        }
        let direct = serde_json::to_string(&Example::Leaf { value: 7 }).unwrap();
        let wrapped = serde_json::to_string(&Spanned::new(
            Example::Leaf { value: 7 },
            Span::new(0, 0, 9),
        ))
        .unwrap();
        assert_eq!(direct, wrapped);
        assert!(direct.contains("\"kind\":\"Leaf\""));
    }
}
