use crate::ast::*;
use crate::error::{XError, XResult};
use crate::source::Spanned;
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Default)]
pub struct CGen {
    lines: Vec<String>,
    indent: usize,
    scopes: Vec<HashMap<String, TypeNode>>,
    temp_counter: usize,
    /// Return type of the function currently being generated (for constructing
    /// Some/None/Ok/Err in `return` position).
    fn_return: Option<TypeNode>,
    /// User-defined struct names (so `c_type` recognises them as value types).
    struct_names: HashSet<String>,
}

impl CGen {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn generate(mut self, program: &Program) -> XResult<String> {
        for item in &program.items {
            if let Item::StructDecl { name, .. } = &item.node {
                self.struct_names.insert(name.clone());
            }
        }
        self.emit("#include <stdint.h>");
        self.emit("#include <stdbool.h>");
        self.emit("#include <stddef.h>");
        self.emit("#include <stdio.h>");
        self.emit("#include <string.h>");
        self.emit("#include <stdlib.h>");
        self.emit("");
        self.emit_runtime_preamble();
        self.emit_networking_preamble();

        for typedef in self.collect_runtime_typedefs(program)? {
            self.emit(&typedef);
        }
        if !self.lines.last().is_some_and(|line| line.is_empty()) {
            self.emit("");
        }

        for item in &program.items {
            if let Item::StructDecl { .. } = &item.node {
                self.gen_struct(&item.node)?;
                self.emit("");
            }
        }

        // Forward declarations so functions can reference each other in any
        // source order (a prerequisite for multi-file module merging too).
        for item in &program.items {
            if let Item::FnDecl { .. } = &item.node {
                self.gen_fn_prototype(&item.node)?;
            }
        }
        self.emit("");

        for item in &program.items {
            match &item.node {
                Item::FnDecl { .. } => {
                    self.gen_fn(&item.node)?;
                    self.emit("");
                }
                Item::StructDecl { .. } | Item::TypeAliasDecl { .. } => {}
            }
        }

        Ok(format!("{}\n", self.lines.join("\n").trim_end()))
    }

    fn emit(&mut self, line: &str) {
        self.lines
            .push(format!("{}{}", "    ".repeat(self.indent), line));
    }

    fn collect_runtime_typedefs(&self, program: &Program) -> XResult<Vec<String>> {
        let mut typedefs = BTreeMap::new();
        for item in &program.items {
            match &item.node {
                Item::StructDecl { fields, .. } => {
                    for field in fields {
                        self.collect_type_typedefs(&field.ty, &mut typedefs)?;
                    }
                }
                Item::TypeAliasDecl { ty, .. } => {
                    self.collect_type_typedefs(ty, &mut typedefs)?;
                }
                Item::FnDecl {
                    params,
                    return_type,
                    body,
                    ..
                } => {
                    self.collect_type_typedefs(return_type, &mut typedefs)?;
                    for param in params {
                        self.collect_type_typedefs(&param.ty, &mut typedefs)?;
                    }
                    self.collect_block_typedefs(body, &mut typedefs)?;
                }
            }
        }
        Ok(typedefs.into_values().collect())
    }

    fn collect_block_typedefs(
        &self,
        block: &Block,
        typedefs: &mut BTreeMap<String, String>,
    ) -> XResult<()> {
        for stmt in &block.statements {
            match &stmt.node {
                Stmt::LetStmt { ty, .. } => self.collect_type_typedefs(ty, typedefs)?,
                Stmt::IfStmt {
                    then_block,
                    else_branch,
                    ..
                } => {
                    self.collect_block_typedefs(then_block, typedefs)?;
                    match else_branch {
                        Some(ElseBranch::Block(block)) => {
                            self.collect_block_typedefs(block, typedefs)?;
                        }
                        Some(ElseBranch::IfStmt(stmt)) => {
                            self.collect_stmt_typedefs(stmt, typedefs)?;
                        }
                        None => {}
                    }
                }
                Stmt::ForStmt { body, .. } | Stmt::WhileStmt { body, .. } => {
                    self.collect_block_typedefs(body, typedefs)?;
                }
                Stmt::MatchStmt { arms, .. } => {
                    for arm in arms {
                        self.collect_block_typedefs(&arm.body, typedefs)?;
                    }
                }
                Stmt::ReturnStmt { .. }
                | Stmt::BreakStmt
                | Stmt::ContinueStmt
                | Stmt::ExprStmt { .. } => {}
            }
        }
        Ok(())
    }

    fn collect_stmt_typedefs(
        &self,
        stmt: &Spanned<Stmt>,
        typedefs: &mut BTreeMap<String, String>,
    ) -> XResult<()> {
        self.collect_block_typedefs(
            &Block {
                kind: "Block",
                statements: vec![stmt.clone()],
            },
            typedefs,
        )
    }

    fn collect_type_typedefs(
        &self,
        ty: &TypeNode,
        typedefs: &mut BTreeMap<String, String>,
    ) -> XResult<()> {
        let TypeNode::TypeExpr { name, args } = ty else {
            return Ok(());
        };
        for arg in args {
            self.collect_type_typedefs(arg, typedefs)?;
        }
        if name == "Slice" {
            if args.len() != 1 {
                return Err(XError::Codegen(format!(
                    "Slice expects exactly one type argument, got {}",
                    args.len()
                )));
            }
            let elem_ty = &args[0];
            let alias = self.c_type(ty)?;
            let elem_c_type = self.c_type(elem_ty)?;
            typedefs.entry(alias.clone()).or_insert_with(|| {
                format!("typedef struct {{\n    {elem_c_type} *data;\n    size_t len;\n}} {alias};")
            });
        }
        if name == "Array" {
            if args.len() != 2 {
                return Err(XError::Codegen(format!(
                    "Array expects exactly two type arguments, got {}",
                    args.len()
                )));
            }
            let elem_ty = &args[0];
            let len = self.const_type_arg_value(&args[1], "Array length")?;
            let alias = self.c_type(ty)?;
            let elem_c_type = self.c_type(elem_ty)?;
            typedefs.entry(alias.clone()).or_insert_with(|| {
                format!("typedef struct {{\n    {elem_c_type} data[{len}];\n}} {alias};")
            });
        }
        if name == "Option" {
            if args.len() != 1 {
                return Err(XError::Codegen(format!(
                    "Option expects exactly one type argument, got {}",
                    args.len()
                )));
            }
            let payload_ty = &args[0];
            let alias = self.c_type(ty)?;
            let payload_c = self.c_type(payload_ty)?;
            typedefs.entry(alias.clone()).or_insert_with(|| {
                format!("typedef struct {{\n    bool some;\n    {payload_c} value;\n}} {alias};")
            });
        }
        if name == "Result" {
            if args.len() != 2 {
                return Err(XError::Codegen(format!(
                    "Result expects exactly two type arguments, got {}",
                    args.len()
                )));
            }
            let ok_ty = &args[0];
            let err_ty = &args[1];
            let alias = self.c_type(ty)?;
            let ok_c = self.c_type(ok_ty)?;
            let err_c = self.c_type(err_ty)?;
            typedefs.entry(alias.clone()).or_insert_with(|| {
                format!(
                    "typedef struct {{\n    bool ok;\n    {ok_c} value;\n    {err_c} error;\n}} {alias};"
                )
            });
        }
        if name == "Vec" {
            if args.len() != 1 {
                return Err(XError::Codegen(format!(
                    "Vec expects exactly one type argument, got {}",
                    args.len()
                )));
            }
            let elem_ty = &args[0];
            let alias = self.c_type(ty)?;
            let elem_c = self.c_type(elem_ty)?;
            let elem_suffix = self.c_type_suffix(elem_ty)?;
            typedefs.entry(alias.clone()).or_insert_with(|| {
                format!(
                    "typedef struct {{\n    {elem_c} *data;\n    size_t len;\n    size_t cap;\n}} {alias};"
                )
            });
            let push_name = format!("__xlang_vec_push_{elem_suffix}");
            typedefs.entry(push_name.clone()).or_insert_with(|| {
                format!(
                    "void {push_name}({alias} *v, {elem_c} x) {{\n    if (v->len == v->cap) {{\n        v->cap = v->cap ? v->cap * 2 : 4;\n        v->data = ({elem_c} *)realloc(v->data, v->cap * sizeof({elem_c}));\n    }}\n    v->data[v->len++] = x;\n}}"
                )
            });
        }
        Ok(())
    }

    fn c_type(&self, ty: &TypeNode) -> XResult<String> {
        match ty {
            TypeNode::TypeExpr { name, args } if args.is_empty() => match name.as_str() {
                "i32" => Ok("int32_t".to_string()),
                "i64" => Ok("int64_t".to_string()),
                "f32" => Ok("float".to_string()),
                "f64" => Ok("double".to_string()),
                "bool" => Ok("bool".to_string()),
                "String" | "Str" => Ok("const char *".to_string()),
                other if self.struct_names.contains(other) => Ok(other.to_string()),
                other => Err(XError::Codegen(format!(
                    "C backend does not support type yet: {other}"
                ))),
            },
            TypeNode::TypeExpr { name, args } if name == "Slice" && args.len() == 1 => {
                Ok(format!("Slice_{}", self.c_type_suffix(&args[0])?))
            }
            TypeNode::TypeExpr { name, args } if name == "Array" && args.len() == 2 => Ok(format!(
                "Array_{}_{}",
                self.c_type_suffix(&args[0])?,
                self.const_type_arg_value(&args[1], "Array length")?
            )),
            TypeNode::TypeExpr { name, args } if name == "Option" && args.len() == 1 => {
                Ok(format!("Option_{}", self.c_type_suffix(&args[0])?))
            }
            TypeNode::TypeExpr { name, args } if name == "Result" && args.len() == 2 => {
                Ok(format!(
                    "Result_{}_{}",
                    self.c_type_suffix(&args[0])?,
                    self.c_type_suffix(&args[1])?
                ))
            }
            TypeNode::TypeExpr { name, args } if name == "Vec" && args.len() == 1 => {
                Ok(format!("Vec_{}", self.c_type_suffix(&args[0])?))
            }
            TypeNode::TypeExpr { name, .. } => Err(XError::Codegen(format!(
                "C backend does not support generic type yet: {name}<...>"
            ))),
            TypeNode::ConstTypeArg { value } => Err(XError::Codegen(format!(
                "unexpected const type argument in C type position: {value}"
            ))),
        }
    }

    fn c_type_suffix(&self, ty: &TypeNode) -> XResult<String> {
        match ty {
            TypeNode::TypeExpr { name, args } if args.is_empty() => match name.as_str() {
                "i32" | "i64" | "f32" | "f64" | "bool" | "String" | "Str" => Ok(name.clone()),
                other if self.struct_names.contains(other) => Ok(other.to_string()),
                other => Err(XError::Codegen(format!(
                    "C backend does not support {other} as a generated type suffix yet"
                ))),
            },
            TypeNode::TypeExpr { name, args } if name == "Slice" && args.len() == 1 => {
                Ok(format!("Slice_{}", self.c_type_suffix(&args[0])?))
            }
            TypeNode::TypeExpr { name, args } if name == "Array" && args.len() == 2 => Ok(format!(
                "Array_{}_{}",
                self.c_type_suffix(&args[0])?,
                self.const_type_arg_value(&args[1], "Array length")?
            )),
            TypeNode::TypeExpr { name, args } if name == "Option" && args.len() == 1 => {
                Ok(format!("Option_{}", self.c_type_suffix(&args[0])?))
            }
            TypeNode::TypeExpr { name, args } if name == "Result" && args.len() == 2 => {
                Ok(format!(
                    "Result_{}_{}",
                    self.c_type_suffix(&args[0])?,
                    self.c_type_suffix(&args[1])?
                ))
            }
            TypeNode::TypeExpr { name, args } if name == "Vec" && args.len() == 1 => {
                Ok(format!("Vec_{}", self.c_type_suffix(&args[0])?))
            }
            TypeNode::TypeExpr { name, .. } => Err(XError::Codegen(format!(
                "C backend does not support {name}<...> as a generated type suffix yet"
            ))),
            TypeNode::ConstTypeArg { value } => Err(XError::Codegen(format!(
                "unexpected const type argument in C type suffix: {value}"
            ))),
        }
    }

    fn const_type_arg_value<'a>(&self, ty: &'a TypeNode, label: &str) -> XResult<&'a str> {
        match ty {
            TypeNode::ConstTypeArg { value } => Ok(value),
            TypeNode::TypeExpr { name, .. } => Err(XError::Codegen(format!(
                "{label} must be a constant integer, got type {name}"
            ))),
        }
    }

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn declare_var(&mut self, name: &str, ty: TypeNode) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string(), ty);
        }
    }

    fn lookup_var(&self, name: &str) -> Option<&TypeNode> {
        self.scopes.iter().rev().find_map(|scope| scope.get(name))
    }

    fn next_temp(&mut self, prefix: &str) -> String {
        let id = self.temp_counter;
        self.temp_counter += 1;
        format!("__xlang_{prefix}{id}")
    }

    fn gen_struct(&mut self, item: &Item) -> XResult<()> {
        let Item::StructDecl { name, fields } = item else {
            unreachable!();
        };
        self.emit(&format!("typedef struct {name} {{"));
        self.indent += 1;
        for field in fields {
            self.emit(&format!("{} {};", self.c_type(&field.ty)?, field.name));
        }
        self.indent -= 1;
        self.emit(&format!("}} {name};"));
        Ok(())
    }

    /// Emit a forward declaration so functions may appear in any source order.
    fn gen_fn_prototype(&mut self, item: &Item) -> XResult<()> {
        let Item::FnDecl {
            name,
            params,
            return_type,
            ..
        } = item
        else {
            unreachable!();
        };
        let ret = self.c_type(return_type)?;
        let params_text = if name == "main" && params.is_empty() {
            "int argc, char** argv".to_string()
        } else if params.is_empty() {
            "void".to_string()
        } else {
            let mut parts = Vec::new();
            for param in params {
                parts.push(format!("{} {}", self.c_type(&param.ty)?, param.name));
            }
            parts.join(", ")
        };
        self.emit(&format!("{ret} {name}({params_text});"));
        Ok(())
    }

    fn gen_fn(&mut self, item: &Item) -> XResult<()> {
        let Item::FnDecl {
            name,
            params,
            return_type,
            body,
        } = item
        else {
            unreachable!();
        };
        let ret = self.c_type(return_type)?;
        let is_main = name == "main" && params.is_empty();
        let params_text = if is_main {
            "int argc, char** argv".to_string()
        } else if params.is_empty() {
            "void".to_string()
        } else {
            let mut parts = Vec::new();
            for param in params {
                parts.push(format!("{} {}", self.c_type(&param.ty)?, param.name));
            }
            parts.join(", ")
        };
        self.emit(&format!("{ret} {name}({params_text}) {{"));
        self.indent += 1;
        self.push_scope();
        for param in params {
            self.declare_var(&param.name, param.ty.clone());
        }
        if is_main {
            self.emit("__xlang_argc_g = argc;");
            self.emit("__xlang_argv_g = argv;");
        }
        self.fn_return = Some(return_type.clone());
        for stmt in &body.statements {
            self.gen_stmt(stmt)?;
        }
        self.fn_return = None;
        self.pop_scope();
        self.indent -= 1;
        self.emit("}");
        Ok(())
    }

    fn gen_stmt(&mut self, stmt: &Spanned<Stmt>) -> XResult<()> {
        match &stmt.node {
            Stmt::LetStmt {
                name, ty, value, ..
            } => self.gen_let_stmt(name, ty, value)?,
            Stmt::ReturnStmt { value } => match value {
                Some(expr) => {
                    let rendered = if let Some(ret_ty) = &self.fn_return {
                        match self.try_constructor(ret_ty, expr)? {
                            Some(c) => c,
                            None => self.gen_expr(expr)?,
                        }
                    } else {
                        self.gen_expr(expr)?
                    };
                    self.emit(&format!("return {rendered};"));
                }
                None => self.emit("return;"),
            },
            Stmt::IfStmt {
                condition,
                then_block,
                else_branch,
            } => {
                self.emit(&format!("if ({}) {{", self.gen_expr(condition)?));
                self.indent += 1;
                for inner in &then_block.statements {
                    self.gen_stmt(inner)?;
                }
                self.indent -= 1;
                match else_branch {
                    None => self.emit("}"),
                    Some(ElseBranch::Block(block)) => {
                        self.emit("} else {");
                        self.indent += 1;
                        for inner in &block.statements {
                            self.gen_stmt(inner)?;
                        }
                        self.indent -= 1;
                        self.emit("}");
                    }
                    Some(ElseBranch::IfStmt(if_stmt)) => {
                        self.emit("} else {");
                        self.indent += 1;
                        self.gen_stmt(if_stmt)?;
                        self.indent -= 1;
                        self.emit("}");
                    }
                }
            }
            Stmt::WhileStmt { condition, body } => {
                self.emit(&format!("while ({}) {{", self.gen_expr(condition)?));
                self.indent += 1;
                self.push_scope();
                for inner in &body.statements {
                    self.gen_stmt(inner)?;
                }
                self.pop_scope();
                self.indent -= 1;
                self.emit("}");
            }
            Stmt::ForStmt {
                iterator,
                iterable,
                body,
            } => self.gen_for_stmt(iterator, iterable, body)?,
            Stmt::ExprStmt { expr } => self.emit(&format!("{};", self.gen_expr(expr)?)),
            Stmt::BreakStmt => self.emit("break;"),
            Stmt::ContinueStmt => self.emit("continue;"),
            Stmt::MatchStmt { value, arms } => self.gen_match_stmt(value, arms)?,
        }
        Ok(())
    }

    /// If `value` is a Some/None/Ok/Err constructor for the Option/Result `ty`,
    /// render the C compound literal; otherwise return `None`.
    fn try_constructor(&self, ty: &TypeNode, value: &Spanned<Expr>) -> XResult<Option<String>> {
        let TypeNode::TypeExpr { name, args } = ty else {
            return Ok(None);
        };
        let alias = self.c_type(ty)?;
        match (name.as_str(), args.len()) {
            ("Option", 1) => match &value.node {
                Expr::CallExpr {
                    callee,
                    args: cargs,
                } if matches!(&callee.node, Expr::Identifier { name: n } if n == "Some")
                    && cargs.len() == 1 =>
                {
                    let v = self.gen_expr(&cargs[0])?;
                    Ok(Some(format!("({alias}){{ .some = true, .value = {v} }}")))
                }
                Expr::Identifier { name: n } if n == "None" => {
                    Ok(Some(format!("({alias}){{ .some = false }}")))
                }
                _ => Ok(None),
            },
            ("Result", 2) => match &value.node {
                Expr::CallExpr {
                    callee,
                    args: cargs,
                } if matches!(&callee.node, Expr::Identifier { name: n } if n == "Ok")
                    && cargs.len() == 1 =>
                {
                    let v = self.gen_expr(&cargs[0])?;
                    Ok(Some(format!("({alias}){{ .ok = true, .value = {v} }}")))
                }
                Expr::CallExpr {
                    callee,
                    args: cargs,
                } if matches!(&callee.node, Expr::Identifier { name: n } if n == "Err")
                    && cargs.len() == 1 =>
                {
                    let v = self.gen_expr(&cargs[0])?;
                    Ok(Some(format!("({alias}){{ .ok = false, .error = {v} }}")))
                }
                _ => Ok(None),
            },
            ("Vec", 1) => match &value.node {
                Expr::CallExpr {
                    callee,
                    args: cargs,
                } if matches!(&callee.node, Expr::Identifier { name: n } if n == "vec_new")
                    && cargs.is_empty() =>
                {
                    Ok(Some(format!(
                        "({alias}){{ .data = 0, .len = 0, .cap = 0 }}"
                    )))
                }
                _ => Ok(None),
            },
            _ => Ok(None),
        }
    }

    fn gen_let_stmt(&mut self, name: &str, ty: &TypeNode, value: &Spanned<Expr>) -> XResult<()> {
        if let Expr::ArrayLiteral { elements } = &value.node {
            self.gen_array_let_stmt(name, ty, elements)?;
        } else if let Some(rendered) = self.try_constructor(ty, value)? {
            self.emit(&format!("{} {} = {};", self.c_type(ty)?, name, rendered));
        } else {
            self.emit(&format!(
                "{} {} = {};",
                self.c_type(ty)?,
                name,
                self.gen_expr(value)?
            ));
        }
        self.declare_var(name, ty.clone());
        Ok(())
    }

    fn gen_array_let_stmt(
        &mut self,
        name: &str,
        ty: &TypeNode,
        elements: &[Spanned<Expr>],
    ) -> XResult<()> {
        let TypeNode::TypeExpr {
            name: ty_name,
            args,
        } = ty
        else {
            return Err(XError::Codegen(
                "array literal initializer requires an Array<T, N> type annotation".to_string(),
            ));
        };
        if ty_name != "Array" || args.len() != 2 {
            return Err(XError::Codegen(format!(
                "array literal initializer requires Array<T, N>, got {ty_name}<...>"
            )));
        }

        let declared_len = self.const_type_arg_value(&args[1], "Array length")?;
        let declared_len = declared_len.parse::<usize>().map_err(|_| {
            XError::Codegen(format!(
                "Array length must fit usize for codegen, got {declared_len:?}"
            ))
        })?;
        if elements.len() != declared_len {
            return Err(XError::Codegen(format!(
                "Array literal length mismatch: Array expects {declared_len} elements, got {}",
                elements.len()
            )));
        }

        let mut rendered_elements = Vec::new();
        for element in elements {
            rendered_elements.push(self.gen_expr(element)?);
        }
        self.emit(&format!(
            "{} {} = {{ .data = {{{}}} }};",
            self.c_type(ty)?,
            name,
            rendered_elements.join(", ")
        ));
        Ok(())
    }

    fn gen_for_stmt(
        &mut self,
        iterator: &str,
        iterable: &Spanned<Expr>,
        body: &Block,
    ) -> XResult<()> {
        let Expr::Identifier {
            name: iterable_name,
        } = &iterable.node
        else {
            return Err(XError::Codegen(
                "C backend only supports `for value in values` where values is an identifier"
                    .to_string(),
            ));
        };

        let Some(TypeNode::TypeExpr { name, args }) = self.lookup_var(iterable_name) else {
            return Err(XError::Codegen(format!(
                "unknown iterable {iterable_name:?} in for loop"
            )));
        };
        let iter_c = self.gen_expr(iterable)?;
        // Loop bound + element source differ: Slice uses a runtime `.len`;
        // Array<T, N> uses the compile-time N. Both store elements in `.data`.
        let (elem_ty, bound, data) = match (name.as_str(), args.len()) {
            ("Slice", 1) => (
                args[0].clone(),
                format!("{iter_c}.len"),
                format!("{iter_c}.data"),
            ),
            ("Array", 2) => {
                let n = self.const_type_arg_value(&args[1], "Array length")?;
                (args[0].clone(), n.to_string(), format!("{iter_c}.data"))
            }
            ("Vec", 1) => (
                args[0].clone(),
                format!("{iter_c}.len"),
                format!("{iter_c}.data"),
            ),
            _ => {
                return Err(XError::Codegen(format!(
                    "C backend only supports for-in over Slice<T> or Array<T, N>, got {name}<...>"
                )));
            }
        };
        let elem_c_type = self.c_type(&elem_ty)?;
        let index = self.next_temp("i");

        self.emit(&format!(
            "for (size_t {index} = 0; {index} < {bound}; {index}++) {{"
        ));
        self.indent += 1;
        self.push_scope();
        self.declare_var(iterator, elem_ty);
        self.emit(&format!("{elem_c_type} {iterator} = {data}[{index}];"));
        for inner in &body.statements {
            self.gen_stmt(inner)?;
        }
        self.pop_scope();
        self.indent -= 1;
        self.emit("}");
        Ok(())
    }

    /// Lower `match scrut { Some/Ok(v) => .., None/Err(..) => .. }` to a C
    /// `if/else` on the discriminant. v1: `scrut` must be a variable of type
    /// `Option<T>` or `Result<T, E>`.
    fn gen_match_stmt(&mut self, value: &Spanned<Expr>, arms: &[MatchArm]) -> XResult<()> {
        let Expr::Identifier { name: scrut_name } = &value.node else {
            return Err(XError::Codegen(
                "match currently supports only a variable (identifier) scrutinee".to_string(),
            ));
        };
        let Some(TypeNode::TypeExpr {
            name: ty_name,
            args,
        }) = self.lookup_var(scrut_name).cloned()
        else {
            return Err(XError::Codegen(format!(
                "match scrutinee {scrut_name:?} is not a typed variable"
            )));
        };
        let is_option = match (ty_name.as_str(), args.len()) {
            ("Option", 1) => true,
            ("Result", 2) => false,
            _ => {
                return Err(XError::Codegen(format!(
                    "match supports Option<T> / Result<T, E>, got {ty_name}"
                )));
            }
        };
        let discriminant = if is_option { "some" } else { "ok" };
        let payload_ty = args[0].clone();
        let err_ty = if is_option {
            None
        } else {
            Some(args[1].clone())
        };

        let mut positive: Option<&MatchArm> = None;
        let mut negative: Option<&MatchArm> = None;
        for arm in arms {
            let Pattern::VariantPattern { name, .. } = &arm.pattern;
            match name.as_str() {
                "Some" | "Ok" => positive = Some(arm),
                "None" | "Err" => negative = Some(arm),
                other => {
                    return Err(XError::Codegen(format!(
                        "C backend does not support match variant {other:?}"
                    )));
                }
            }
        }

        let scrut_c = self.gen_expr(value)?;
        self.emit(&format!("if ({scrut_c}.{discriminant}) {{"));
        self.indent += 1;
        self.push_scope();
        if let Some(arm) = positive {
            let Pattern::VariantPattern { bindings, .. } = &arm.pattern;
            if let Some(binding) = bindings.first() {
                let payload_c = self.c_type(&payload_ty)?;
                self.declare_var(binding, payload_ty.clone());
                self.emit(&format!("{payload_c} {binding} = {scrut_c}.value;"));
            }
            for inner in &arm.body.statements {
                self.gen_stmt(inner)?;
            }
        }
        self.pop_scope();
        self.indent -= 1;
        if let Some(arm) = negative {
            self.emit("} else {");
            self.indent += 1;
            self.push_scope();
            if let Some(err_ty) = &err_ty {
                let Pattern::VariantPattern { bindings, .. } = &arm.pattern;
                if let Some(binding) = bindings.first() {
                    let err_c = self.c_type(err_ty)?;
                    self.declare_var(binding, err_ty.clone());
                    self.emit(&format!("{err_c} {binding} = {scrut_c}.error;"));
                }
            }
            for inner in &arm.body.statements {
                self.gen_stmt(inner)?;
            }
            self.pop_scope();
            self.indent -= 1;
        }
        self.emit("}");
        Ok(())
    }

    /// Recognise the print builtins (`print_i32`/`print_f64`/`print_str`/
    /// `print_bool`) and lower a one-arg call to a `printf`; returns None for
    /// anything else so the normal call path handles it.
    /// Emit the small C runtime preamble — helpers that need allocation (string
    /// concatenation, int->str). Non-static so an unused helper doesn't trip
    /// -Wunused-function.
    fn emit_runtime_preamble(&mut self) {
        let lines = [
            "int __xlang_argc_g = 0;",
            "char** __xlang_argv_g = 0;",
            "char* __xlang_str_concat(const char* a, const char* b) {",
            "    size_t la = strlen(a), lb = strlen(b);",
            "    char* out = (char*)malloc(la + lb + 1);",
            "    memcpy(out, a, la);",
            "    memcpy(out + la, b, lb);",
            "    out[la + lb] = 0;",
            "    return out;",
            "}",
            "char* __xlang_int_to_str(int32_t n) {",
            "    char* buf = (char*)malloc(16);",
            "    snprintf(buf, 16, \"%d\", n);",
            "    return buf;",
            "}",
            "char* __xlang_read_stdin() {",
            "    size_t cap = 65536, len = 0;",
            "    char* buf = (char*)malloc(cap);",
            "    size_t r;",
            "    while ((r = fread(buf + len, 1, cap - len, stdin)) > 0) {",
            "        len += r;",
            "        if (len + 1 >= cap) { cap *= 2; buf = (char*)realloc(buf, cap); }",
            "    }",
            "    buf[len] = 0;",
            "    return buf;",
            "}",
            "char* __xlang_read_file(const char* path) {",
            "    FILE* f = fopen(path, \"rb\");",
            "    if (!f) { char* e = (char*)malloc(1); e[0] = 0; return e; }",
            "    fseek(f, 0, SEEK_END); long sz = ftell(f); fseek(f, 0, SEEK_SET);",
            "    char* buf = (char*)malloc(sz + 1);",
            "    size_t rd = fread(buf, 1, sz, f); (void)rd; buf[sz] = 0; fclose(f);",
            "    return buf;",
            "}",
            "void __xlang_write_file(const char* path, const char* content) {",
            "    FILE* f = fopen(path, \"wb\");",
            "    if (!f) return;",
            "    fwrite(content, 1, strlen(content), f); fclose(f);",
            "}",
            "int32_t __xlang_str_find(const char* s, const char* sub) {",
            "    const char* p = strstr(s, sub);",
            "    return p ? (int32_t)(p - s) : -1;",
            "}",
            "char* __xlang_str_slice(const char* s, int32_t start, int32_t end) {",
            "    int32_t n = (int32_t)strlen(s);",
            "    if (start < 0) start = 0;",
            "    if (end > n) end = n;",
            "    if (end < start) end = start;",
            "    int32_t len = end - start;",
            "    char* out = (char*)malloc(len + 1);",
            "    memcpy(out, s + start, len); out[len] = 0;",
            "    return out;",
            "}",
            "char* __xlang_str_reverse(const char* s) {",
            "    int32_t n = (int32_t)strlen(s);",
            "    char* out = (char*)malloc(n + 1);",
            "    for (int32_t i = 0; i < n; i++) out[i] = s[n - 1 - i];",
            "    out[n] = 0;",
            "    return out;",
            "}",
            "char* __xlang_str_translate(const char* s, const char* from, const char* to) {",
            "    int32_t n = (int32_t)strlen(s);",
            "    int32_t tn = (int32_t)strlen(to);",
            "    char* out = (char*)malloc(n + 1);",
            "    for (int32_t i = 0; i < n; i++) {",
            "        char* p = strchr(from, s[i]);",
            "        out[i] = (p && (p - from) < tn) ? to[p - from] : s[i];",
            "    }",
            "    out[n] = 0;",
            "    return out;",
            "}",
            "",
        ];
        for line in lines {
            self.emit(line);
        }
    }

    /// Networking helpers (socket I/O), guarded so non-Linux builds (which lack
    /// these POSIX headers) skip them entirely. Programs use networking only on
    /// Linux (CI / the target server); on Windows the block is preprocessed out,
    /// so the run-safe tests (which cc the generated C locally) still pass.
    fn emit_networking_preamble(&mut self) {
        let lines = [
            "#if !defined(_WIN32)",
            "#include <unistd.h>",
            "#include <sys/socket.h>",
            "#include <netinet/in.h>",
            "#include <arpa/inet.h>",
            "#include <dirent.h>",
            "int32_t __xlang_tcp_listen(int32_t port) {",
            "    int fd = socket(AF_INET, SOCK_STREAM, 0);",
            "    int opt = 1;",
            "    setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &opt, sizeof(opt));",
            "    struct sockaddr_in addr;",
            "    addr.sin_family = AF_INET;",
            "    addr.sin_addr.s_addr = INADDR_ANY;",
            "    addr.sin_port = htons((uint16_t)port);",
            "    bind(fd, (struct sockaddr*)&addr, sizeof(addr));",
            "    listen(fd, 64);",
            "    return (int32_t)fd;",
            "}",
            "char* __xlang_recv_str(int32_t fd) {",
            "    char* buf = (char*)malloc(65536);",
            "    ssize_t n = recv(fd, buf, 65535, 0);",
            "    if (n < 0) n = 0;",
            "    buf[n] = 0;",
            "    return buf;",
            "}",
            "int32_t __xlang_dir_count(const char* path) {",
            "    DIR* d = opendir(path);",
            "    if (!d) return 0;",
            "    int32_t n = 0;",
            "    while (readdir(d)) n++;",
            "    closedir(d);",
            "    return n;",
            "}",
            "char* __xlang_dir_entry(const char* path, int32_t idx) {",
            "    DIR* d = opendir(path);",
            "    if (!d) return \"\";",
            "    struct dirent* e;",
            "    int32_t i = 0;",
            "    while ((e = readdir(d))) {",
            "        if (i == idx) {",
            "            char* copy = (char*)malloc(strlen(e->d_name) + 1);",
            "            strcpy(copy, e->d_name);",
            "            closedir(d);",
            "            return copy;",
            "        }",
            "        i++;",
            "    }",
            "    closedir(d);",
            "    return \"\";",
            "}",
            "#endif",
            "",
        ];
        for line in lines {
            self.emit(line);
        }
    }

    /// Lower the string builtins `str_len` / `str_concat` / `int_to_str`
    /// (strlen inline; the other two call the runtime-preamble helpers).
    fn try_string_call(
        &self,
        callee: &Spanned<Expr>,
        args: &[Spanned<Expr>],
    ) -> XResult<Option<String>> {
        let Expr::Identifier { name } = &callee.node else {
            return Ok(None);
        };
        let Some(first) = args.first() else {
            return Ok(None);
        };
        let a = self.gen_expr(first)?;
        let rendered = match name.as_str() {
            "str_len" => format!("(int32_t)strlen({a})"),
            "argv" => format!("__xlang_argv_g[{a}]"),
            "print_raw" => format!("printf(\"%s\", {a})"),
            "int_to_str" => format!("__xlang_int_to_str({a})"),
            "str_concat" => {
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                format!("__xlang_str_concat({a}, {b})")
            }
            "str_eq" => {
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                format!("(strcmp({a}, {b}) == 0)")
            }
            "str_find" => {
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                format!("__xlang_str_find({a}, {b})")
            }
            "str_slice" => {
                if args.len() < 3 {
                    return Ok(None);
                }
                let b = self.gen_expr(&args[1])?;
                let c = self.gen_expr(&args[2])?;
                format!("__xlang_str_slice({a}, {b}, {c})")
            }
            "str_reverse" => format!("__xlang_str_reverse({a})"),
            "str_translate" => {
                let (Some(second), Some(third)) = (args.get(1), args.get(2)) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                let c = self.gen_expr(third)?;
                format!("__xlang_str_translate({a}, {b}, {c})")
            }
            "str_char_at" => {
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                format!("(int32_t)(unsigned char)({a}[{b}])")
            }
            "dir_count" => format!("__xlang_dir_count({a})"),
            "dir_entry" => {
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                format!("__xlang_dir_entry({a}, {b})")
            }
            "str_cmp" => {
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                format!("(int32_t)strcmp({a}, {b})")
            }
            "vec_len" => format!("((int32_t)({a}).len)"),
            "str_to_int" => format!("(int32_t)strtol({a}, 0, 10)"),
            "remove_file" => format!("remove({a})"),
            "rename_file" => {
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                format!("rename({a}, {b})")
            }
            "read_file" => format!("__xlang_read_file({a})"),
            "write_file" => {
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                format!("__xlang_write_file({a}, {b})")
            }
            "tcp_listen" => format!("__xlang_tcp_listen({a})"),
            "accept" => format!("accept({a}, 0, 0)"),
            "recv_str" => format!("__xlang_recv_str({a})"),
            "close_fd" => format!("close({a})"),
            "send_str" => {
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                format!("send({a}, {b}, strlen({b}), 0)")
            }
            _ => return Ok(None),
        };
        Ok(Some(rendered))
    }

    /// Lower `v.push(x)` on a `Vec<T>` variable to a call of the per-element
    /// runtime helper `__xlang_vec_push_T(&v, x)` (emitted in the typedef pass).
    fn try_vec_push_call(
        &self,
        callee: &Spanned<Expr>,
        args: &[Spanned<Expr>],
    ) -> XResult<Option<String>> {
        let Expr::FieldAccessExpr { object, field } = &callee.node else {
            return Ok(None);
        };
        if field != "push" || args.len() != 1 {
            return Ok(None);
        }
        let Expr::Identifier { name: vname } = &object.node else {
            return Ok(None);
        };
        let Some(TypeNode::TypeExpr { name, args: targs }) = self.lookup_var(vname) else {
            return Ok(None);
        };
        if name != "Vec" || targs.len() != 1 {
            return Ok(None);
        }
        let elem_suffix = self.c_type_suffix(&targs[0])?;
        let v_c = self.gen_expr(object)?;
        let x_c = self.gen_expr(&args[0])?;
        Ok(Some(format!(
            "__xlang_vec_push_{elem_suffix}(&{v_c}, {x_c})"
        )))
    }

    /// Zero-argument builtins (`fork`, `getpid`) — lower to the C calls. They
    /// need <unistd.h>, which the guarded networking preamble includes on Linux.
    fn try_zero_arg_call(
        &self,
        callee: &Spanned<Expr>,
        args: &[Spanned<Expr>],
    ) -> XResult<Option<String>> {
        if !args.is_empty() {
            return Ok(None);
        }
        let Expr::Identifier { name } = &callee.node else {
            return Ok(None);
        };
        Ok(Some(match name.as_str() {
            "fork" => "fork()".to_string(),
            "getpid" => "getpid()".to_string(),
            "argc" => "(__xlang_argc_g)".to_string(),
            "read_stdin" => "__xlang_read_stdin()".to_string(),
            _ => return Ok(None),
        }))
    }

    fn try_print_call(
        &self,
        callee: &Spanned<Expr>,
        args: &[Spanned<Expr>],
    ) -> XResult<Option<String>> {
        let Expr::Identifier { name } = &callee.node else {
            return Ok(None);
        };
        if args.len() != 1 {
            return Ok(None);
        }
        self.try_print_builtin(name, &args[0])
    }

    fn try_print_builtin(&self, name: &str, arg: &Spanned<Expr>) -> XResult<Option<String>> {
        let arg_c = self.gen_expr(arg)?;
        let rendered = match name {
            "print_i32" => format!("printf(\"%d\\n\", {arg_c})"),
            "print_f64" => format!("printf(\"%f\\n\", {arg_c})"),
            "print_str" => format!("printf(\"%s\\n\", {arg_c})"),
            "print_bool" => format!("printf(\"%s\\n\", ({arg_c}) ? \"true\" : \"false\")"),
            _ => return Ok(None),
        };
        Ok(Some(rendered))
    }

    fn gen_expr(&self, expr: &Spanned<Expr>) -> XResult<String> {
        match &expr.node {
            Expr::IntLiteral { value } | Expr::FloatLiteral { value } => Ok(value.clone()),
            Expr::StringLiteral { value } => Ok(serde_json::to_string(value)?),
            Expr::BoolLiteral { value } => Ok(if *value { "true" } else { "false" }.to_string()),
            Expr::Identifier { name } => Ok(name.clone()),
            Expr::ArrayLiteral { .. } => Err(XError::Codegen(
                "array literals are only supported in typed Array<T, N> let initializers"
                    .to_string(),
            )),
            Expr::BinaryExpr { op, left, right } => Ok(format!(
                "({} {} {})",
                self.gen_expr(left)?,
                op,
                self.gen_expr(right)?
            )),
            Expr::UnaryExpr { op, value } => Ok(format!("({}{})", op, self.gen_expr(value)?)),
            Expr::AssignmentExpr { target, value } => Ok(format!(
                "({} = {})",
                self.gen_expr(target)?,
                self.gen_expr(value)?
            )),
            Expr::CallExpr { callee, args } => {
                if let Some(rendered) = self.try_zero_arg_call(callee, args)? {
                    return Ok(rendered);
                }
                if let Some(rendered) = self.try_print_call(callee, args)? {
                    return Ok(rendered);
                }
                if let Some(rendered) = self.try_string_call(callee, args)? {
                    return Ok(rendered);
                }
                if let Some(rendered) = self.try_vec_push_call(callee, args)? {
                    return Ok(rendered);
                }
                let mut parts = Vec::new();
                for arg in args {
                    parts.push(self.gen_expr(arg)?);
                }
                Ok(format!("{}({})", self.gen_expr(callee)?, parts.join(", ")))
            }
            Expr::FieldAccessExpr { object, field } => {
                Ok(format!("{}.{}", self.gen_expr(object)?, field))
            }
            Expr::StructLiteral { name, fields } => {
                let mut parts = Vec::new();
                for f in fields {
                    parts.push(format!(".{} = {}", f.name, self.gen_expr(&f.value)?));
                }
                Ok(format!("({name}){{ {} }}", parts.join(", ")))
            }
            Expr::IndexExpr { object, index } => {
                // Both Array<T,N> and Slice<T> store elements in `.data`, so
                // indexing lowers uniformly.
                Ok(format!(
                    "{}.data[{}]",
                    self.gen_expr(object)?,
                    self.gen_expr(index)?
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CGen;
    use crate::lexer::Lexer;
    use crate::parser::Parser;

    fn gen_c(source: &str) -> String {
        let (tokens, _) = Lexer::new(source).tokenize();
        let program = Parser::new(tokens, "<test>").parse().expect("parse source");
        CGen::new().generate(&program).expect("codegen")
    }

    #[test]
    fn lowers_option_match_to_if_else() {
        let c = gen_c(
            "module main\nfn f(o: Option<i32>): i32 { match o { Some(v) => { return v } None => { return 0 } } }\nfn main(): i32 { return 0 }",
        );
        assert!(c.contains("typedef struct"), "no Option struct: {c}");
        assert!(c.contains(".some"), "no .some field: {c}");
        assert!(c.contains("if (o.some)"), "no match lowering: {c}");
    }

    #[test]
    fn lowers_result_match_to_if_else() {
        let c = gen_c(
            "module main\nfn f(r: Result<i32, String>): i32 { match r { Ok(v) => { return v } Err(e) => { return 0 } } }\nfn main(): i32 { return 0 }",
        );
        assert!(c.contains(".ok"), "no .ok field: {c}");
        assert!(c.contains("if (r.ok)"), "no result match lowering: {c}");
    }

    #[test]
    fn emits_struct_literal_compound() {
        let c = gen_c(
            "module main\nstruct P { x: i32 }\nfn main(): i32 { let p: P = P { x: 1 } return p.x }",
        );
        assert!(c.contains("(P){ .x ="), "no struct literal: {c}");
    }

    #[test]
    fn emits_vec_push_helper_call() {
        let c = gen_c(
            "module main\nfn main(): i32 { let mut v: Vec<i32> = vec_new() v.push(1) return 0 }",
        );
        assert!(c.contains("__xlang_vec_push_i32(&v,"), "no vec push: {c}");
    }

    #[test]
    fn emits_fork_call() {
        let c = gen_c("module main\nfn main(): i32 { let p: i32 = fork() return p }");
        assert!(c.contains("fork();"), "no fork: {c}");
    }

    #[test]
    fn lowers_for_in_over_array() {
        let c = gen_c(
            "module main\nfn main(): i32 { let a: Array<i32, 3> = [1, 2, 3] for n in a { print_i32(n) } return 0 }",
        );
        assert!(c.contains("< 3;"), "no array bound N: {c}");
        assert!(c.contains(".data["), "no .data index: {c}");
    }

    #[test]
    fn emits_print_printf() {
        let c = gen_c("module main\nfn main(): i32 { print_i32(42) return 0 }");
        assert!(c.contains("printf("), "no printf: {c}");
    }

    #[test]
    fn emits_array_literal_initializer() {
        let c = gen_c("module main\nfn main(): i32 { let a: Array<i32, 2> = [1, 2] return 0 }");
        assert!(c.contains(".data = {"), "no array literal init: {c}");
    }

    #[test]
    fn emits_function_prototype() {
        let c = gen_c(
            "module main\nfn helper(x: i32): i32 { return x }\nfn main(): i32 { return helper(1) }",
        );
        assert!(
            c.contains("int32_t helper(int32_t x);"),
            "no prototype: {c}"
        );
    }

    #[test]
    fn emits_str_eq_as_strcmp() {
        let c = gen_c(
            "module main\nfn f(a: String, b: String): bool { return str_eq(a, b) }\nfn main(): i32 { return 0 }",
        );
        assert!(c.contains("strcmp("), "no strcmp for str_eq: {c}");
    }

    #[test]
    fn emits_str_find_and_slice_helpers() {
        let c = gen_c(
            "module main\nfn main(): i32 { let s: String = \"hi\" let i: i32 = str_find(s, \"h\") let t: String = str_slice(s, 0, 1) return 0 }",
        );
        assert!(c.contains("__xlang_str_find("), "no str_find: {c}");
        assert!(c.contains("__xlang_str_slice("), "no str_slice: {c}");
    }
}
