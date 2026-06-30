//! Language-server logic — testable and separate from the stdio JSON-RPC loop
//! in `src/bin/xlang-lsp.rs`. Built on the Phase 1 diagnostics and Phase 2
//! symbol index. Provides: live diagnostics, hover (signature at a position),
//! go-to-definition (the defining range), and completion (top-level names).

use crate::driver::{diagnostics_to_serializable, parse_source};
use crate::symbols::{self, Range, SymbolIndex};

/// All diagnostics for `source`, in LSP-serializable form (1-based line/col +
/// raw byte span), for `textDocument/publishDiagnostics`.
pub fn diagnostics(source: &str, file: &str) -> Vec<crate::driver::SerializableDiagnostic> {
    let (_program, diags) = parse_source(source, file);
    diagnostics_to_serializable(&diags, source, file)
}

/// The symbol (function or struct) whose definition range contains `(line,col)`,
/// returning its signature (hover) and its range (go-to-definition).
pub fn symbol_at(source: &str, file: &str, line: u32, col: u32) -> Option<(String, Range)> {
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
            ));
        }
    }
    for s in &index.structs {
        if contains(&s.range, line, col) {
            return Some((
                format!("struct {} {{ {} }}", s.name, s.fields.join(", ")),
                s.range.clone(),
            ));
        }
    }
    None
}

/// Hover text (signature) at a 1-based `(line, col)`.
pub fn hover(source: &str, file: &str, line: u32, col: u32) -> Option<String> {
    symbol_at(source, file, line, col).map(|(sig, _)| sig)
}

/// The defining range at a 1-based `(line, col)` — for go-to-definition.
pub fn definition(source: &str, file: &str, line: u32, col: u32) -> Option<Range> {
    symbol_at(source, file, line, col).map(|(_, range)| range)
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
}
