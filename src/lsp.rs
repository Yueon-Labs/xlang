//! Language-server logic — testable and separate from the stdio JSON-RPC loop
//! in `src/bin/xlang-lsp.rs`. Built on the Phase 1 diagnostics and Phase 2
//! symbol index. Provides: live diagnostics, hover (signature at a position),
//! go-to-definition (the defining range), and completion (top-level names).

use crate::driver::{diagnostics_to_serializable, parse_source};
use crate::source::LineIndex;
use crate::symbols::{self, Range, SymbolIndex};

/// All diagnostics for `source`, in LSP-serializable form (1-based line/col +
/// raw byte span), for `textDocument/publishDiagnostics`.
pub fn diagnostics(source: &str, file: &str) -> Vec<crate::driver::SerializableDiagnostic> {
    let (_program, diags) = parse_source(source, file);
    diagnostics_to_serializable(&diags, source, file)
}

/// The symbol (function or struct) whose definition range contains `(line,col)`,
/// returning its signature (hover), its range (go-to-definition), and its doc.
pub fn symbol_at(
    source: &str,
    file: &str,
    line: u32,
    col: u32,
) -> Option<(String, Range, Option<String>)> {
    let (program, _diags) = parse_source(source, file);
    let program = program?;
    let index: SymbolIndex = symbols::build_index(&program.items, source);
    for f in &index.functions {
        if contains(&f.range, line, col) {
            return Some((
                format!(
                    "fn {}({}) -> {}",
                    f.name,
                    f.params.join(", "),
                    f.return_type
                ),
                f.range.clone(),
                f.doc.clone(),
            ));
        }
    }
    for s in &index.structs {
        if contains(&s.range, line, col) {
            return Some((
                format!("struct {} {{ {} }}", s.name, s.fields.join(", ")),
                s.range.clone(),
                s.doc.clone(),
            ));
        }
    }
    None
}

/// Hover text (signature + doc) at a 1-based `(line, col)`.
pub fn hover(source: &str, file: &str, line: u32, col: u32) -> Option<String> {
    symbol_at(source, file, line, col).map(|(sig, _, doc)| match doc {
        Some(d) if !d.is_empty() => format!("{sig}\n\n{d}"),
        _ => sig,
    })
}

/// The defining range at a 1-based `(line, col)` — for go-to-definition.
pub fn definition(source: &str, file: &str, line: u32, col: u32) -> Option<Range> {
    symbol_at(source, file, line, col).map(|(_, range, _)| range)
}

/// Top-level names for completion (all functions + structs).
pub fn completion_names(source: &str, file: &str) -> Vec<String> {
    let (program, _diags) = parse_source(source, file);
    let program = match program {
        Some(p) => p,
        None => return Vec::new(),
    };
    let index = symbols::build_index(&program.items, source);
    let mut names: Vec<String> = index.functions.iter().map(|f| f.name.clone()).collect();
    names.extend(index.structs.iter().map(|s| s.name.clone()));
    names.sort();
    names.dedup();
    names
}

fn contains(r: &Range, line: u32, col: u32) -> bool {
    let after_start = line > r.line || (line == r.line && col >= r.col);
    let before_end = line < r.end_line || (line == r.end_line && col <= r.end_col);
    after_start && before_end
}

/// A document-outline entry for `textDocument/documentSymbol`: a top-level
/// name, its LSP SymbolKind (Function = 12, Struct = 23), and its range.
pub struct DocumentSymbolEntry {
    pub name: String,
    pub kind: u32,
    pub range: Range,
}

/// Top-level functions + structs (in source order) for the editor outline.
pub fn document_symbols(source: &str, file: &str) -> Vec<DocumentSymbolEntry> {
    let (program, _diags) = parse_source(source, file);
    let program = match program {
        Some(p) => p,
        None => return Vec::new(),
    };
    let index = symbols::build_index(&program.items, source);
    let mut out: Vec<DocumentSymbolEntry> = index
        .functions
        .iter()
        .map(|f| DocumentSymbolEntry {
            name: f.name.clone(),
            kind: 12, // Function
            range: f.range.clone(),
        })
        .collect();
    out.extend(index.structs.iter().map(|s| DocumentSymbolEntry {
        name: s.name.clone(),
        kind: 23, // Struct
        range: s.range.clone(),
    }));
    out
}

/// Whole-word occurrences of the identifier at 1-based `(line, col)`, as ranges
/// — for `textDocument/references` (and documentHighlight). Text-based (not
/// AST-resolved): accurate for top-level symbols (functions/structs/types), may
/// over-match same-named locals in unrelated scopes. The definition occurrence
/// is included. Returns an empty vec if the cursor isn't on an identifier.
pub fn references(source: &str, _file: &str, line: u32, col: u32) -> Vec<Range> {
    let idx = LineIndex::new(source);
    let bytes = source.as_bytes();
    let is_word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let at = (idx.byte_offset(line, col) as usize).min(bytes.len().saturating_sub(1));
    // Extend left/right from the cursor to the identifier extent.
    let mut s = at;
    while s > 0 && is_word(bytes[s - 1]) {
        s -= 1;
    }
    let mut e = at;
    while e < bytes.len() && is_word(bytes[e]) {
        e += 1;
    }
    if s == e {
        return Vec::new();
    }
    let needle = &source[s..e];
    let nlen = (e - s) as u32;
    let n = bytes.len();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < n {
        match source[i..].find(needle) {
            Some(rel) => {
                let start = i + rel;
                let end = start + needle.len();
                let left_ok = start == 0 || !is_word(bytes[start - 1]);
                let right_ok = end == n || !is_word(bytes[end]);
                if left_ok && right_ok {
                    let (sl, sc) = idx.line_col(start as u32);
                    out.push(Range {
                        line: sl as u32,
                        col: sc as u32,
                        end_line: sl as u32,
                        end_col: sc as u32 + nlen,
                    });
                }
                i = end;
            }
            None => break,
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = "module main\nfn add(a: i32, b: i32): i32 {\n    return a + b\n}\n";

    #[test]
    fn hover_returns_function_signature() {
        // "fn add" is on line 2; hovering anywhere on that line yields the sig.
        let h = hover(SRC, "<t>", 2, 5).expect("hover present");
        assert_eq!(h, "fn add(a: i32, b: i32) -> i32");
    }

    #[test]
    fn definition_returns_the_defining_range() {
        let r = definition(SRC, "<t>", 2, 5).expect("definition present");
        assert_eq!(r.line, 2);
        assert_eq!(r.col, 1);
    }

    #[test]
    fn hover_outside_any_symbol_is_none() {
        assert!(hover(SRC, "<t>", 99, 1).is_none());
    }

    #[test]
    fn completion_lists_functions_and_structs() {
        let src = "module main\nstruct P {\n    x: i32\n}\nfn f(): i32 {\n    return 0\n}\n";
        let names = completion_names(src, "<t>");
        assert!(names.contains(&"f".to_string()));
        assert!(names.contains(&"P".to_string()));
    }

    #[test]
    fn diagnostics_catch_a_type_error() {
        let src = "module main\nfn main(): i32 {\n    let x: i32 = true\n    return x\n}\n";
        let ds = diagnostics(src, "<t>");
        assert!(!ds.is_empty(), "expected a type-mismatch diagnostic");
    }

    #[test]
    fn document_symbols_lists_functions_and_structs() {
        let src = "module main\nstruct Point {\n    x: i32\n    y: i32\n}\nfn add(a: i32, b: i32): i32 { return a + b }\nfn main(): i32 { return 0 }";
        let syms = document_symbols(src, "<t>");
        let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Point"), "struct missing: {names:?}");
        assert!(names.contains(&"add"), "fn add missing: {names:?}");
        assert!(names.contains(&"main"), "fn main missing: {names:?}");
        assert_eq!(syms.iter().find(|s| s.name == "Point").unwrap().kind, 23); // Struct
        assert_eq!(syms.iter().find(|s| s.name == "add").unwrap().kind, 12); // Function
    }

    #[test]
    fn references_finds_all_occurrences_of_a_function() {
        // add is declared once and called twice (whole-word, not inside "address").
        let src = "module main\nfn add(a: i32, b: i32): i32 { return a + b }\nfn address(): i32 { return 0 }\nfn main(): i32 { return add(1, 2) + add(3, 4) }";
        // cursor on the `add` in the declaration (line 2, col 4).
        let refs = references(src, "<t>", 2, 4);
        assert_eq!(
            refs.len(),
            3,
            "should find decl + 2 calls, not inside 'address'"
        );
    }
}

#[cfg(test)]
mod doc_tests {
    use super::*;
    #[test]
    fn hover_includes_doc_comment() {
        let src = "module main\n/// Doubles x.\n/// Second line.\nfn double(x: i32): i32 {\n    return x * 2\n}\n";
        let h = hover(src, "<t>", 4, 5).expect("hover present");
        assert!(
            h.contains("fn double(x: i32) -> i32"),
            "hover should include signature: {h}"
        );
        assert!(h.contains("Doubles x."), "hover should include doc: {h}");
        assert!(
            h.contains("Second line."),
            "hover should include multi-line doc: {h}"
        );
    }
}
