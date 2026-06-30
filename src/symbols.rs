//! Symbol index — a summary of a program's top-level definitions (functions,
//! structs) with source positions. This is the data source for a future LSP
//! (hover, go-to-definition, completion): `xlangc symbols <file>` emits it as
//! JSON.

use serde::Serialize;

use crate::ast::{Item, TypeNode};
use crate::source::{LineIndex, Span, Spanned};

/// 1-based source range (line/col..endLine/endCol), LSP-style.
#[derive(Clone, Serialize)]
pub struct Range {
    pub line: u32,
    pub col: u32,
    #[serde(rename = "endLine")]
    pub end_line: u32,
    #[serde(rename = "endCol")]
    pub end_col: u32,
}

#[derive(Serialize)]
pub struct FunctionSymbol {
    pub name: String,
    pub params: Vec<String>,
    #[serde(rename = "returnType")]
    pub return_type: String,
    pub range: Range,
}

#[derive(Serialize)]
pub struct StructSymbol {
    pub name: String,
    pub fields: Vec<String>,
    pub range: Range,
}

#[derive(Serialize)]
pub struct SymbolIndex {
    pub functions: Vec<FunctionSymbol>,
    pub structs: Vec<StructSymbol>,
}

/// Render a TypeNode as an xlang-style type string: `i32`, `String`,
/// `Vec<i32>`, `Array<Student, 3>`, etc.
pub fn type_to_str(ty: &TypeNode) -> String {
    match ty {
        TypeNode::TypeExpr { name, args } => {
            if args.is_empty() {
                name.clone()
            } else {
                let rendered: Vec<String> = args.iter().map(type_to_str).collect();
                format!("{}<{}>", name, rendered.join(", "))
            }
        }
        TypeNode::ConstTypeArg { value } => value.clone(),
    }
}

fn range_of(span: &Span, lines: &LineIndex) -> Range {
    let (line, col) = lines.line_col(span.start);
    let (end_line, end_col) = lines.line_col(span.end);
    Range {
        line: line as u32,
        col: col as u32,
        end_line: end_line as u32,
        end_col: end_col as u32,
    }
}

/// Build a symbol index from a program's top-level items and its source text.
pub fn build_index(items: &[Spanned<Item>], source: &str) -> SymbolIndex {
    let lines = LineIndex::new(source);
    let mut functions = Vec::new();
    let mut structs = Vec::new();
    for item in items {
        let range = range_of(&item.span, &lines);
        match &item.node {
            Item::FnDecl {
                name,
                params,
                return_type,
                ..
            } => {
                let ps: Vec<String> = params
                    .iter()
                    .map(|p| format!("{}: {}", p.name, type_to_str(&p.ty)))
                    .collect();
                functions.push(FunctionSymbol {
                    name: name.clone(),
                    params: ps,
                    return_type: type_to_str(return_type),
                    range,
                });
            }
            Item::StructDecl { name, fields, .. } => {
                let fs: Vec<String> = fields
                    .iter()
                    .map(|f| format!("{}: {}", f.name, type_to_str(&f.ty)))
                    .collect();
                structs.push(StructSymbol {
                    name: name.clone(),
                    fields: fs,
                    range,
                });
            }
            Item::TypeAliasDecl { .. } => {}
        }
    }
    SymbolIndex { functions, structs }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;
    use crate::parser::Parser;

    fn index_of(src: &str) -> SymbolIndex {
        let (tokens, _) = Lexer::new(src).tokenize();
        let program = Parser::new(tokens, "<t>").parse().expect("parse");
        build_index(&program.items, src)
    }

    #[test]
    fn extracts_function_with_params_and_range() {
        let src = "module main\nfn add(a: i32, b: i32): i32 {\n    return a + b\n}\n";
        let idx = index_of(src);
        assert_eq!(idx.functions.len(), 1);
        let f = &idx.functions[0];
        assert_eq!(f.name, "add");
        assert_eq!(f.params, vec!["a: i32".to_string(), "b: i32".to_string()]);
        assert_eq!(f.return_type, "i32");
        assert_eq!(f.range.line, 2);
        assert_eq!(f.range.col, 1);
    }

    #[test]
    fn extracts_struct_with_fields() {
        let src = "module main\nstruct Point {\n    x: i32\n    y: i32\n}\n";
        let idx = index_of(src);
        assert_eq!(idx.structs.len(), 1);
        let s = &idx.structs[0];
        assert_eq!(s.name, "Point");
        assert_eq!(s.fields, vec!["x: i32".to_string(), "y: i32".to_string()]);
        assert_eq!(s.range.line, 2);
    }

    #[test]
    fn renders_generic_types() {
        let src = "module main\nfn f(xs: Vec<i32>): Vec<String> {\n    return xs\n}\n";
        let idx = index_of(src);
        assert_eq!(idx.functions[0].params, vec!["xs: Vec<i32>".to_string()]);
        assert_eq!(idx.functions[0].return_type, "Vec<String>");
    }
}
