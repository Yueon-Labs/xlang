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
        self.emit("#include <time.h>");
        self.emit("#include <locale.h>");
        self.emit("");
        self.emit_runtime_preamble();
        self.emit_networking_preamble();

        // User struct definitions first, so wrapper typedefs (Array/Vec/...)
        // that reference them (e.g. Array<Student, 3>) see a complete type.
        for item in &program.items {
            if let Item::StructDecl { .. } = &item.node {
                self.gen_struct(&item.node)?;
                self.emit("");
            }
        }

        for typedef in self.collect_runtime_typedefs(program)? {
            self.emit(&typedef);
        }
        if !self.lines.last().is_some_and(|line| line.is_empty()) {
            self.emit("");
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
        // Emit wrapper typedefs in dependency order (fixpoint): a wrapper whose
        // definition references another not-yet-emitted wrapper must wait. This
        // fixes nested wrappers (e.g. Array<Array<i32>, 3> needs Array_i32 first)
        // which BTreeMap's alphabetical order would emit backwards.
        let mut pending = typedefs;
        let mut ordered: Vec<String> = Vec::new();
        let mut emitted: std::collections::HashSet<String> = std::collections::HashSet::new();
        while !pending.is_empty() {
            let names: Vec<String> = pending.keys().cloned().collect();
            let mut progressed = false;
            for name in &names {
                let Some(def) = pending.get(name) else {
                    continue;
                };
                let blocked = pending
                    .keys()
                    .any(|other| other != name && def.contains(other) && !emitted.contains(other));
                if !blocked {
                    ordered.push(def.clone());
                    emitted.insert(name.clone());
                    pending.remove(name);
                    progressed = true;
                }
            }
            if !progressed {
                break;
            }
        }
        for def in pending.into_values() {
            ordered.push(def);
        }
        Ok(ordered)
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
            "// SHA-256 hash → 64-char hex string. Standard FIPS 180-4 implementation.",
            "char* __xlang_sha256_hex(const char* data) {",
            "    static const uint32_t K[64] = {",
            "        0x428a2f98,0x71374491,0xb5c0fbcf,0xe9b5dba5,0x3956c25b,0x59f111f1,0x923f82a4,0xab1c5ed5,",
            "        0xd807aa98,0x12835b01,0x243185be,0x550c7dc3,0x72be5d74,0x80deb1fe,0x9bdc06a7,0xc19bf174,",
            "        0xe49b69c1,0xefbe4786,0x0fc19dc6,0x240ca1cc,0x2de92c6f,0x4a7484aa,0x5cb0a9dc,0x76f988da,",
            "        0x983e5152,0xa831c66d,0xb00327c8,0xbf597fc7,0xc6e00bf3,0xd5a79147,0x06ca6351,0x14292967,",
            "        0x27b70a85,0x2e1b2138,0x4d2c6dfc,0x53380d13,0x650a7354,0x766a0abb,0x81c2c92e,0x92722c85,",
            "        0xa2bfe8a1,0xa81a664b,0xc24b8b70,0xc76c51a3,0xd192e819,0xd6990624,0xf40e3585,0x106aa070,",
            "        0x19a4c116,0x1e376c08,0x2748774c,0x34b0bcb5,0x391c0cb3,0x4ed8aa4a,0x5b9cca4f,0x682e6ff3,",
            "        0x748f82ee,0x78a5636f,0x84c87814,0x8cc70208,0x90befffa,0xa4506ceb,0xbef9a3f7,0xc67178f2",
            "    };",
            "    uint32_t h[8] = {0x6a09e667,0xbb67ae85,0x3c6ef372,0xa54ff53a,0x510e527f,0x9b05688c,0x1f83d9ab,0x5be0cd19};",
            "    size_t dlen = strlen(data);",
            "    size_t padded = ((dlen + 9 + 63) / 64) * 64;",
            "    uint8_t* msg = (uint8_t*)calloc(padded, 1);",
            "    memcpy(msg, data, dlen);",
            "    msg[dlen] = 0x80;",
            "    uint64_t bits = (uint64_t)dlen * 8;",
            "    for (int i = 0; i < 8; i++) msg[padded - 1 - i] = (uint8_t)(bits >> (i * 8));",
            "    for (size_t off = 0; off < padded; off += 64) {",
            "        uint32_t w[64];",
            "        for (int i = 0; i < 16; i++) w[i] = ((uint32_t)msg[off+i*4]<<24)|((uint32_t)msg[off+i*4+1]<<16)|((uint32_t)msg[off+i*4+2]<<8)|((uint32_t)msg[off+i*4+3]);",
            "        for (int i = 16; i < 64; i++) {",
            "            uint32_t s0 = ((w[i-15]>>7)|(w[i-15]<<25)) ^ ((w[i-15]>>18)|(w[i-15]<<14)) ^ (w[i-15]>>3);",
            "            uint32_t s1 = ((w[i-2]>>17)|(w[i-2]<<15)) ^ ((w[i-2]>>19)|(w[i-2]<<13)) ^ (w[i-2]>>10);",
            "            w[i] = w[i-16] + s0 + w[i-7] + s1;",
            "        }",
            "        uint32_t a=h[0],b=h[1],c=h[2],d=h[3],e=h[4],f=h[5],g=h[6],hh=h[7];",
            "        for (int i = 0; i < 64; i++) {",
            "            uint32_t S1 = ((e>>6)|(e<<26)) ^ ((e>>11)|(e<<21)) ^ ((e>>25)|(e<<7));",
            "            uint32_t ch = (e & f) ^ (~e & g);",
            "            uint32_t t1 = hh + S1 + ch + K[i] + w[i];",
            "            uint32_t S0 = ((a>>2)|(a<<30)) ^ ((a>>13)|(a<<19)) ^ ((a>>22)|(a<<10));",
            "            uint32_t maj = (a & b) ^ (a & c) ^ (b & c);",
            "            uint32_t t2 = S0 + maj;",
            "            hh=g; g=f; f=e; e=d+t1; d=c; c=b; b=a; a=t1+t2;",
            "        }",
            "        h[0]+=a; h[1]+=b; h[2]+=c; h[3]+=d; h[4]+=e; h[5]+=f; h[6]+=g; h[7]+=hh;",
            "    }",
            "    free(msg);",
            "    char* hex = (char*)malloc(65);",
            "    const char* hc = \"0123456789abcdef\";",
            "    for (int i = 0; i < 8; i++) { hex[i*8]=(char)hc[(h[i]>>28)&15]; hex[i*8+1]=(char)hc[(h[i]>>24)&15]; hex[i*8+2]=(char)hc[(h[i]>>20)&15]; hex[i*8+3]=(char)hc[(h[i]>>16)&15]; hex[i*8+4]=(char)hc[(h[i]>>12)&15]; hex[i*8+5]=(char)hc[(h[i]>>8)&15]; hex[i*8+6]=(char)hc[(h[i]>>4)&15]; hex[i*8+7]=(char)hc[h[i]&15]; }",
            "    hex[64] = 0;",
            "    return hex;",
            "}",
            "// SHA-224 hash (FIPS 180-4) — SHA-256 with different IV, truncated to 56 hex chars.",
            "char* __xlang_sha224_hex(const char* data) {",
            "    static const uint32_t K[64] = {",
            "        0x428a2f98,0x71374491,0xb5c0fbcf,0xe9b5dba5,0x3956c25b,0x59f111f1,0x923f82a4,0xab1c5ed5,",
            "        0xd807aa98,0x12835b01,0x243185be,0x550c7dc3,0x72be5d74,0x80deb1fe,0x9bdc06a7,0xc19bf174,",
            "        0xe49b69c1,0xefbe4786,0x0fc19dc6,0x240ca1cc,0x2de92c6f,0x4a7484aa,0x5cb0a9dc,0x76f988da,",
            "        0x983e5152,0xa831c66d,0xb00327c8,0xbf597fc7,0xc6e00bf3,0xd5a79147,0x06ca6351,0x14292967,",
            "        0x27b70a85,0x2e1b2138,0x4d2c6dfc,0x53380d13,0x650a7354,0x766a0abb,0x81c2c92e,0x92722c85,",
            "        0xa2bfe8a1,0xa81a664b,0xc24b8b70,0xc76c51a3,0xd192e819,0xd6990624,0xf40e3585,0x106aa070,",
            "        0x19a4c116,0x1e376c08,0x2748774c,0x34b0bcb5,0x391c0cb3,0x4ed8aa4a,0x5b9cca4f,0x682e6ff3,",
            "        0x748f82ee,0x78a5636f,0x84c87814,0x8cc70208,0x90befffa,0xa4506ceb,0xbef9a3f7,0xc67178f2",
            "    };",
            "    uint32_t h[8]={0xc1059ed8,0x367cd507,0x3070dd17,0xf70e5939,0xffc00b31,0x68581511,0x64f98fa7,0xbefa4fa4};",
            "    size_t dlen=strlen(data),padded=((dlen+9+63)/64)*64;",
            "    uint8_t* msg=(uint8_t*)calloc(padded,1);",
            "    memcpy(msg,data,dlen);",
            "    msg[dlen]=0x80;",
            "    uint64_t bits=(uint64_t)dlen*8;",
            "    for(int i=0;i<8;i++) msg[padded-1-i]=(uint8_t)(bits>>(i*8));",
            "    for(size_t off=0;off<padded;off+=64){",
            "        uint32_t w[64];",
            "        for(int i=0;i<16;i++) w[i]=((uint32_t)msg[off+i*4]<<24)|((uint32_t)msg[off+i*4+1]<<16)|((uint32_t)msg[off+i*4+2]<<8)|((uint32_t)msg[off+i*4+3]);",
            "        for(int i=16;i<64;i++){uint32_t s0=((w[i-15]>>7)|(w[i-15]<<25))^((w[i-15]>>18)|(w[i-15]<<14))^(w[i-15]>>3);uint32_t s1=((w[i-2]>>17)|(w[i-2]<<15))^((w[i-2]>>19)|(w[i-2]<<13))^(w[i-2]>>10);w[i]=w[i-16]+s0+w[i-7]+s1;}",
            "        uint32_t a=h[0],b=h[1],c=h[2],d=h[3],e=h[4],f=h[5],g=h[6],hh=h[7];",
            "        for(int i=0;i<64;i++){uint32_t S1=((e>>6)|(e<<26))^((e>>11)|(e<<21))^((e>>25)|(e<<7));uint32_t ch=(e&f)^(~e&g);uint32_t t1=hh+S1+ch+K[i]+w[i];uint32_t S0=((a>>2)|(a<<30))^((a>>13)|(a<<19))^((a>>22)|(a<<10));uint32_t maj=(a&b)^(a&c)^(b&c);uint32_t t2=S0+maj;hh=g;g=f;f=e;e=d+t1;d=c;c=b;b=a;a=t1+t2;}",
            "        h[0]+=a;h[1]+=b;h[2]+=c;h[3]+=d;h[4]+=e;h[5]+=f;h[6]+=g;h[7]+=hh;",
            "    }",
            "    free(msg);",
            "    char* hex=(char*)malloc(57);",
            "    const char* hc=\"0123456789abcdef\";",
            "    for(int i=0;i<7;i++){hex[i*8]=(char)hc[(h[i]>>28)&15];hex[i*8+1]=(char)hc[(h[i]>>24)&15];hex[i*8+2]=(char)hc[(h[i]>>20)&15];hex[i*8+3]=(char)hc[(h[i]>>16)&15];hex[i*8+4]=(char)hc[(h[i]>>12)&15];hex[i*8+5]=(char)hc[(h[i]>>8)&15];hex[i*8+6]=(char)hc[(h[i]>>4)&15];hex[i*8+7]=(char)hc[h[i]&15];}",
            "    hex[56]=0;",
            "    return hex;",
            "}",
            "char* __xlang_pad_int(int32_t n, int32_t width) {",
            "    char* buf = (char*)malloc(32);",
            "    snprintf(buf, 32, \"%*d\", width, n);",
            "    return buf;",
            "}",
            "// SHA-512 hash (FIPS 180-4) → 128-char hex string.",
            "char* __xlang_sha512_hex(const char* data) {",
            "    static const uint64_t K[80]={",
            "        0x428a2f98d728ae22ULL,0x7137449123ef65cdULL,0xb5c0fbcfec4d3b2fULL,0xe9b5dba58189dbbcULL,",
            "        0x3956c25bf348b538ULL,0x59f111f1b605d019ULL,0x923f82a4af194f9bULL,0xab1c5ed5da6d8118ULL,",
            "        0xd807aa98a3030242ULL,0x12835b0145706fbeULL,0x243185be4ee4b28cULL,0x550c7dc3d5ffb4e2ULL,",
            "        0x72be5d74f27b896fULL,0x80deb1fe3b1696b1ULL,0x9bdc06a725c71235ULL,0xc19bf174cf692694ULL,",
            "        0xe49b69c19ef14ad2ULL,0xefbe4786384f25e3ULL,0x0fc19dc68b8cd5b5ULL,0x240ca1cc77ac9c65ULL,",
            "        0x2de92c6f592b0275ULL,0x4a7484aa6ea6e483ULL,0x5cb0a9dcbd41fbd4ULL,0x76f988da831153b5ULL,",
            "        0x983e5152ee66dfabULL,0xa831c66d2db43210ULL,0xb00327c898fb213fULL,0xbf597fc7beef0ee4ULL,",
            "        0xc6e00bf33da88fc2ULL,0xd5a79147930aa725ULL,0x06ca6351e003826fULL,0x142929670a0e6e70ULL,",
            "        0x27b70a8546d22ffcULL,0x2e1b21385c26c926ULL,0x4d2c6dfc5ac42aedULL,0x53380d139d95b3dfULL,",
            "        0x650a73548baf63deULL,0x766a0abb3c77b2a8ULL,0x81c2c92e47edaee6ULL,0x92722c851482353bULL,",
            "        0xa2bfe8a14cf10364ULL,0xa81a664bbc423001ULL,0xc24b8b70d0f89791ULL,0xc76c51a30654be30ULL,",
            "        0xd192e819d6ef5218ULL,0xd69906245565a910ULL,0xf40e35855771202aULL,0x106aa07032bbd1b8ULL,",
            "        0x19a4c116b8d2d0c8ULL,0x1e376c085141ab53ULL,0x2748774cdf8eeb99ULL,0x34b0bcb5e19b48a8ULL,",
            "        0x391c0cb3c5c95a63ULL,0x4ed8aa4ae3418acbULL,0x5b9cca4f7763e373ULL,0x682e6ff3d6b2b8a3ULL,",
            "        0x748f82ee5defb2fcULL,0x78a5636f43172f60ULL,0x84c87814a1f0ab72ULL,0x8cc702081a6439ecULL,",
            "        0x90befffa23631e28ULL,0xa4506cebde82bde9ULL,0xbef9a3f7b2c67915ULL,0xc67178f2e372532bULL,",
            "        0xca273eceea26619cULL,0xd186b8c721c0c207ULL,0xeada7dd6cde0eb1eULL,0xf57d4f7fee6ed178ULL,",
            "        0x06f067aa72176fbaULL,0x0a637dc5a2c898a6ULL,0x113f9804bef90daeULL,0x1b710b35131c471bULL,",
            "        0x28db77f523047d84ULL,0x32caab7b40c72493ULL,0x3c9ebe0a15c9bebcULL,0x431d67c49c100d4cULL,",
            "        0x4cc5d4becb3e42b6ULL,0x597f299cfc657e2aULL,0x5fcb6fab3ad6faecULL,0x6c44198c4a475817ULL",
            "    };",
            "    uint64_t h[8]={0x6a09e667f3bcc908ULL,0xbb67ae8584caa73bULL,0x3c6ef372fe94f82bULL,0xa54ff53a5f1d36f1ULL,0x510e527fade682d1ULL,0x9b05688c2b3e6c1fULL,0x1f83d9abfb41bd6bULL,0x5be0cd19137e2179ULL};",
            "    size_t dlen=strlen(data);",
            "    size_t padded=((dlen+17+127)/128)*128;",
            "    uint8_t* msg=(uint8_t*)calloc(padded,1);",
            "    memcpy(msg,data,dlen);",
            "    msg[dlen]=0x80;",
            "    uint64_t bits=(uint64_t)dlen*8;",
            "    for(int i=0;i<8;i++) msg[padded-1-i]=(uint8_t)(bits>>(i*8));",
            "    for(size_t off=0;off<padded;off+=128){",
            "        uint64_t w[80];",
            "        for(int i=0;i<16;i++){size_t b=off+i*8;w[i]=((uint64_t)msg[b]<<56)|((uint64_t)msg[b+1]<<48)|((uint64_t)msg[b+2]<<40)|((uint64_t)msg[b+3]<<32)|((uint64_t)msg[b+4]<<24)|((uint64_t)msg[b+5]<<16)|((uint64_t)msg[b+6]<<8)|((uint64_t)msg[b+7]);}",
            "        for(int i=16;i<80;i++){uint64_t s0=((w[i-15]>>1)|(w[i-15]<<63))^((w[i-15]>>8)|(w[i-15]<<56))^(w[i-15]>>7);uint64_t s1=((w[i-2]>>19)|(w[i-2]<<45))^((w[i-2]>>61)|(w[i-2]<<3))^(w[i-2]>>6);w[i]=w[i-16]+s0+w[i-7]+s1;}",
            "        uint64_t a=h[0],b=h[1],c=h[2],d=h[3],e=h[4],f=h[5],g=h[6],hh=h[7];",
            "        for(int i=0;i<80;i++){",
            "            uint64_t S1=((e>>14)|(e<<50))^((e>>18)|(e<<46))^((e>>41)|(e<<23));",
            "            uint64_t ch=(e&f)^(~e&g);",
            "            uint64_t t1=hh+S1+ch+K[i]+w[i];",
            "            uint64_t S0=((a>>28)|(a<<36))^((a>>34)|(a<<30))^((a>>39)|(a<<25));",
            "            uint64_t maj=(a&b)^(a&c)^(b&c);",
            "            uint64_t t2=S0+maj;",
            "            hh=g;g=f;f=e;e=d+t1;d=c;c=b;b=a;a=t1+t2;",
            "        }",
            "        h[0]+=a;h[1]+=b;h[2]+=c;h[3]+=d;h[4]+=e;h[5]+=f;h[6]+=g;h[7]+=hh;",
            "    }",
            "    free(msg);",
            "    char* hex=(char*)malloc(129);",
            "    const char* hc=\"0123456789abcdef\";",
            "    for(int i=0;i<8;i++){for(int j=7;j>=0;j--){hex[i*16+(7-j)*2]=(char)hc[(h[i]>>(j*8+4))&15];hex[i*16+(7-j)*2+1]=(char)hc[(h[i]>>(j*8))&15];}}",
            "    hex[128]=0;",
            "    return hex;",
            "}",
            "// SHA-384 hash (FIPS 180-4) — SHA-512 with different IV, truncated to 96 hex chars.",
            "char* __xlang_sha384_hex(const char* data) {",
            "    static const uint64_t K384[80]={",
            "        0x428a2f98d728ae22ULL,0x7137449123ef65cdULL,0xb5c0fbcfec4d3b2fULL,0xe9b5dba58189dbbcULL,",
            "        0x3956c25bf348b538ULL,0x59f111f1b605d019ULL,0x923f82a4af194f9bULL,0xab1c5ed5da6d8118ULL,",
            "        0xd807aa98a3030242ULL,0x12835b0145706fbeULL,0x243185be4ee4b28cULL,0x550c7dc3d5ffb4e2ULL,",
            "        0x72be5d74f27b896fULL,0x80deb1fe3b1696b1ULL,0x9bdc06a725c71235ULL,0xc19bf174cf692694ULL,",
            "        0xe49b69c19ef14ad2ULL,0xefbe4786384f25e3ULL,0x0fc19dc68b8cd5b5ULL,0x240ca1cc77ac9c65ULL,",
            "        0x2de92c6f592b0275ULL,0x4a7484aa6ea6e483ULL,0x5cb0a9dcbd41fbd4ULL,0x76f988da831153b5ULL,",
            "        0x983e5152ee66dfabULL,0xa831c66d2db43210ULL,0xb00327c898fb213fULL,0xbf597fc7beef0ee4ULL,",
            "        0xc6e00bf33da88fc2ULL,0xd5a79147930aa725ULL,0x06ca6351e003826fULL,0x142929670a0e6e70ULL,",
            "        0x27b70a8546d22ffcULL,0x2e1b21385c26c926ULL,0x4d2c6dfc5ac42aedULL,0x53380d139d95b3dfULL,",
            "        0x650a73548baf63deULL,0x766a0abb3c77b2a8ULL,0x81c2c92e47edaee6ULL,0x92722c851482353bULL,",
            "        0xa2bfe8a14cf10364ULL,0xa81a664bbc423001ULL,0xc24b8b70d0f89791ULL,0xc76c51a30654be30ULL,",
            "        0xd192e819d6ef5218ULL,0xd69906245565a910ULL,0xf40e35855771202aULL,0x106aa07032bbd1b8ULL,",
            "        0x19a4c116b8d2d0c8ULL,0x1e376c085141ab53ULL,0x2748774cdf8eeb99ULL,0x34b0bcb5e19b48a8ULL,",
            "        0x391c0cb3c5c95a63ULL,0x4ed8aa4ae3418acbULL,0x5b9cca4f7763e373ULL,0x682e6ff3d6b2b8a3ULL,",
            "        0x748f82ee5defb2fcULL,0x78a5636f43172f60ULL,0x84c87814a1f0ab72ULL,0x8cc702081a6439ecULL,",
            "        0x90befffa23631e28ULL,0xa4506cebde82bde9ULL,0xbef9a3f7b2c67915ULL,0xc67178f2e372532bULL,",
            "        0xca273eceea26619cULL,0xd186b8c721c0c207ULL,0xeada7dd6cde0eb1eULL,0xf57d4f7fee6ed178ULL,",
            "        0x06f067aa72176fbaULL,0x0a637dc5a2c898a6ULL,0x113f9804bef90daeULL,0x1b710b35131c471bULL,",
            "        0x28db77f523047d84ULL,0x32caab7b40c72493ULL,0x3c9ebe0a15c9bebcULL,0x431d67c49c100d4cULL,",
            "        0x4cc5d4becb3e42b6ULL,0x597f299cfc657e2aULL,0x5fcb6fab3ad6faecULL,0x6c44198c4a475817ULL",
            "    };",
            "    uint64_t h[8]={0xcbbb9d5dc1059ed8ULL,0x629a292a367cd507ULL,0x9159015a3070dd17ULL,0x152fecd8f70e5939ULL,0x67332667ffc00b31ULL,0x8eb44a8768581511ULL,0xdb0c2e0d64f98fa7ULL,0x47b5481dbefa4fa4ULL};",
            "    size_t dlen=strlen(data),padded=((dlen+17+127)/128)*128;",
            "    uint8_t* msg=(uint8_t*)calloc(padded,1);",
            "    memcpy(msg,data,dlen);",
            "    msg[dlen]=0x80;",
            "    uint64_t bits=(uint64_t)dlen*8;",
            "    for(int i=0;i<8;i++) msg[padded-1-i]=(uint8_t)(bits>>(i*8));",
            "    for(size_t off=0;off<padded;off+=128){",
            "        uint64_t w[80];",
            "        for(int i=0;i<16;i++){size_t b=off+i*8;w[i]=((uint64_t)msg[b]<<56)|((uint64_t)msg[b+1]<<48)|((uint64_t)msg[b+2]<<40)|((uint64_t)msg[b+3]<<32)|((uint64_t)msg[b+4]<<24)|((uint64_t)msg[b+5]<<16)|((uint64_t)msg[b+6]<<8)|((uint64_t)msg[b+7]);}",
            "        for(int i=16;i<80;i++){uint64_t s0=((w[i-15]>>1)|(w[i-15]<<63))^((w[i-15]>>8)|(w[i-15]<<56))^(w[i-15]>>7);uint64_t s1=((w[i-2]>>19)|(w[i-2]<<45))^((w[i-2]>>61)|(w[i-2]<<3))^(w[i-2]>>6);w[i]=w[i-16]+s0+w[i-7]+s1;}",
            "        uint64_t a=h[0],b=h[1],c=h[2],d=h[3],e=h[4],f=h[5],g=h[6],hh=h[7];",
            "        for(int i=0;i<80;i++){uint64_t S1=((e>>14)|(e<<50))^((e>>18)|(e<<46))^((e>>41)|(e<<23));uint64_t ch=(e&f)^(~e&g);uint64_t t1=hh+S1+ch+K384[i]+w[i];uint64_t S0=((a>>28)|(a<<36))^((a>>34)|(a<<30))^((a>>39)|(a<<25));uint64_t maj=(a&b)^(a&c)^(b&c);uint64_t t2=S0+maj;hh=g;g=f;f=e;e=d+t1;d=c;c=b;b=a;a=t1+t2;}",
            "        h[0]+=a;h[1]+=b;h[2]+=c;h[3]+=d;h[4]+=e;h[5]+=f;h[6]+=g;h[7]+=hh;",
            "    }",
            "    free(msg);",
            "    char* hex=(char*)malloc(97);",
            "    const char* hc=\"0123456789abcdef\";",
            "    for(int i=0;i<6;i++){for(int j=7;j>=0;j--){hex[i*16+(7-j)*2]=(char)hc[(h[i]>>(j*8+4))&15];hex[i*16+(7-j)*2+1]=(char)hc[(h[i]>>(j*8))&15];}}",
            "    hex[96]=0;",
            "    return hex;",
            "}",
            "// SHA-1 hash (FIPS 180-4) → 40-char hex string.",
            "char* __xlang_sha1_hex(const char* data) {",
            "    uint32_t h[5]={0x67452301,0xEFCDAB89,0x98BADCFE,0x10325476,0xC3D2E1F0};",
            "    size_t dlen=strlen(data);",
            "    size_t padded=((dlen+9+63)/64)*64;",
            "    uint8_t* msg=(uint8_t*)calloc(padded,1);",
            "    memcpy(msg,data,dlen);",
            "    msg[dlen]=0x80;",
            "    uint64_t bits=(uint64_t)dlen*8;",
            "    for(int i=0;i<8;i++) msg[padded-1-i]=(uint8_t)(bits>>(i*8));",
            "    for(size_t off=0;off<padded;off+=64){",
            "        uint32_t w[80];",
            "        for(int i=0;i<16;i++) w[i]=((uint32_t)msg[off+i*4]<<24)|((uint32_t)msg[off+i*4+1]<<16)|((uint32_t)msg[off+i*4+2]<<8)|((uint32_t)msg[off+i*4+3]);",
            "        for(int i=16;i<80;i++){uint32_t t=w[i-3]^w[i-8]^w[i-14]^w[i-16]; w[i]=(t<<1)|(t>>31);}",
            "        uint32_t a=h[0],b=h[1],c=h[2],d=h[3],e=h[4];",
            "        for(int i=0;i<80;i++){",
            "            uint32_t f,k;",
            "            if(i<20){f=(b&c)|(~b&d);k=0x5A827999;}",
            "            else if(i<40){f=b^c^d;k=0x6ED9EBA1;}",
            "            else if(i<60){f=(b&c)|(b&d)|(c&d);k=0x8F1BBCDC;}",
            "            else{f=b^c^d;k=0xCA62C1D6;}",
            "            uint32_t temp=((a<<5)|(a>>27))+f+e+k+w[i];",
            "            e=d;d=c;c=((b<<30)|(b>>2));b=a;a=temp;",
            "        }",
            "        h[0]+=a;h[1]+=b;h[2]+=c;h[3]+=d;h[4]+=e;",
            "    }",
            "    free(msg);",
            "    char* hex=(char*)malloc(41);",
            "    const char* hc=\"0123456789abcdef\";",
            "    for(int i=0;i<5;i++){hex[i*8]=(char)hc[(h[i]>>28)&15];hex[i*8+1]=(char)hc[(h[i]>>24)&15];hex[i*8+2]=(char)hc[(h[i]>>20)&15];hex[i*8+3]=(char)hc[(h[i]>>16)&15];hex[i*8+4]=(char)hc[(h[i]>>12)&15];hex[i*8+5]=(char)hc[(h[i]>>8)&15];hex[i*8+6]=(char)hc[(h[i]>>4)&15];hex[i*8+7]=(char)hc[h[i]&15];}",
            "    hex[40]=0;",
            "    return hex;",
            "}",
            "// MD5 hash (RFC 1321) → 32-char hex string.",
            "char* __xlang_md5_hex(const char* data) {",
            "    static const uint32_t T[64] = {",
            "        0xd76aa478,0xe8c7b756,0x242070db,0xc1bdceee,0xf57c0faf,0x4787c62a,0xa8304613,0xfd469501,",
            "        0x698098d8,0x8b44f7af,0xffff5bb1,0x895cd7be,0x6b901122,0xfd987193,0xa679438e,0x49b40821,",
            "        0xf61e2562,0xc040b340,0x265e5a51,0xe9b6c7aa,0xd62f105d,0x02441453,0xd8a1e681,0xe7d3fbc8,",
            "        0x21e1cde6,0xc33707d6,0xf4d50d87,0x455a14ed,0xa9e3e905,0xfcefa3f8,0x676f02d9,0x8d2a4c8a,",
            "        0xfffa3942,0x8771f681,0x6d9d6122,0xfde5380c,0xa4beea44,0x4bdecfa9,0xf6bb4b60,0xbebfbc70,",
            "        0x289b7ec6,0xeaa127fa,0xd4ef3085,0x04881d05,0xd9d4d039,0xe6db99e5,0x1fa27cf8,0xc4ac5665,",
            "        0xf4292244,0x432aff97,0xab9423a7,0xfc93a039,0x655b59c3,0x8f0ccc92,0xffeff47d,0x85845dd1,",
            "        0x6fa87e4f,0xfe2ce6e0,0xa3014314,0x4e0811a1,0xf7537e82,0xbd3af235,0x2ad7d2bb,0xeb86d391",
            "    };",
            "    static const int s[64] = {7,12,17,22,7,12,17,22,7,12,17,22,7,12,17,22,5,9,14,20,5,9,14,20,5,9,14,20,5,9,14,20,4,11,16,23,4,11,16,23,4,11,16,23,4,11,16,23,6,10,15,21,6,10,15,21,6,10,15,21,6,10,15,21};",
            "    uint32_t a0=0x67452301,b0=0xefcdab89,c0=0x98badcfe,d0=0x10325476;",
            "    size_t dlen=strlen(data);",
            "    size_t padded=((dlen+9+63)/64)*64;",
            "    uint8_t* msg=(uint8_t*)calloc(padded,1);",
            "    memcpy(msg,data,dlen);",
            "    msg[dlen]=0x80;",
            "    uint64_t bits=(uint64_t)dlen*8;",
            "    for(int i=0;i<8;i++) msg[padded-8+i]=(uint8_t)(bits>>(i*8));",
            "    for(size_t off=0;off<padded;off+=64){",
            "        uint32_t M[16];",
            "        for(int i=0;i<16;i++) M[i]=((uint32_t)msg[off+i*4])|((uint32_t)msg[off+i*4+1]<<8)|((uint32_t)msg[off+i*4+2]<<16)|((uint32_t)msg[off+i*4+3]<<24);",
            "        uint32_t A=a0,B=b0,C=c0,D=d0;",
            "        for(int i=0;i<64;i++){",
            "            uint32_t F; int g;",
            "            if(i<16){F=(B&C)|(~B&D);g=i;}",
            "            else if(i<32){F=(D&B)|(~D&C);g=(5*i+1)%16;}",
            "            else if(i<48){F=B^C^D;g=(3*i+5)%16;}",
            "            else{F=C^(B|~D);g=(7*i)%16;}",
            "            F=F+A+T[i]+M[g];",
            "            A=D;D=C;C=B;",
            "            B=B+((F<<s[i])|(F>>(32-s[i])));",
            "        }",
            "        a0+=A;b0+=B;c0+=C;d0+=D;",
            "    }",
            "    free(msg);",
            "    char* hex=(char*)malloc(33);",
            "    const char* hc=\"0123456789abcdef\";",
            "    uint32_t hh[4]={a0,b0,c0,d0};",
            "    for(int i=0;i<4;i++){for(int j=0;j<4;j++){hex[i*8+j*2]=(char)hc[(hh[i]>>(j*8+4))&15];hex[i*8+j*2+1]=(char)hc[(hh[i]>>(j*8))&15];}}",
            "    hex[32]=0;",
            "    return hex;",
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
            "    size_t cap = 65536, len = 0;",
            "    char* buf = (char*)malloc(cap);",
            "    size_t r;",
            "    while ((r = fread(buf + len, 1, cap - len, f)) > 0) {",
            "        len += r;",
            "        if (len + 1 >= cap) { cap *= 2; buf = (char*)realloc(buf, cap); }",
            "    }",
            "    buf[len] = 0; fclose(f);",
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
            "    if (start < 0) start = 0;",
            "    if (end < start) end = start;",
            "    int32_t len = end - start;",
            "    char* out = (char*)malloc((size_t)len + 1);",
            "    memcpy(out, s + start, (size_t)len); out[len] = 0;",
            "    return out;",
            "}",
            "char* __xlang_str_trim(const char* s) {",
            "    size_t n = strlen(s), a = 0, b = n;",
            "    while (a < b && (s[a] == ' ' || s[a] == '\\t' || s[a] == '\\n' || s[a] == '\\r')) a++;",
            "    while (b > a && (s[b-1] == ' ' || s[b-1] == '\\t' || s[b-1] == '\\n' || s[b-1] == '\\r')) b--;",
            "    size_t len = b - a;",
            "    char* out = (char*)malloc(len + 1);",
            "    memcpy(out, s + a, len); out[len] = 0;",
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
            "char* __xlang_read_line() {",
            "    char* buf = (char*)malloc(65536);",
            "    if (!fgets(buf, 65536, stdin)) { buf[0] = 0; return buf; }",
            "    int32_t n = (int32_t)strlen(buf);",
            "    if (n > 0 && buf[n - 1] == '\\n') buf[n - 1] = 0;",
            "    return buf;",
            "}",
            "static char* __sb_buf = 0;",
            "static size_t __sb_len = 0;",
            "static size_t __sb_cap = 0;",
            "void __xlang_sb_new() {",
            "    if (!__sb_buf) { __sb_buf = (char*)malloc(65536); __sb_cap = 65536; }",
            "    __sb_len = 0; __sb_buf[0] = 0;",
            "}",
            "void __xlang_sb_push(const char* s) {",
            "    size_t sl = strlen(s);",
            "    if (__sb_len + sl + 1 > __sb_cap) {",
            "        while (__sb_len + sl + 1 > __sb_cap) __sb_cap *= 2;",
            "        __sb_buf = (char*)realloc(__sb_buf, __sb_cap);",
            "    }",
            "    memcpy(__sb_buf + __sb_len, s, sl);",
            "    __sb_len += sl;",
            "    __sb_buf[__sb_len] = 0;",
            "}",
            "const char* __xlang_sb_str() {",
            "    return __sb_buf ? __sb_buf : \"\";",
            "}",
            "void __xlang_sb_push_char(int32_t c) {",
            "    if (__sb_len + 2 > __sb_cap) { __sb_cap *= 2; __sb_buf = (char*)realloc(__sb_buf, __sb_cap); }",
            "    __sb_buf[__sb_len++] = (char)c;",
            "    __sb_buf[__sb_len] = 0;",
            "}",
            "char* __xlang_time_str() {",
            "    setlocale(LC_TIME, \"\");",
            "    time_t t = time(NULL);",
            "    struct tm* tm = localtime(&t);",
            "    char* s = (char*)malloc(64);",
            "    strftime(s, 64, \"%a %b %e %H:%M:%S %Z %Y\", tm);",
            "    return s;",
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
            "#include <sys/stat.h>",
            "#include <signal.h>",
            "#include <sys/utsname.h>",
            "#include <sys/epoll.h>",
            "#include <fcntl.h>",
            "#include <sys/sendfile.h>",
            "#include <netinet/tcp.h>",
            "#include <errno.h>",
            "#include <sched.h>",
            "#include <sys/wait.h>",
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
            "    static char buf[65536];",
            "    ssize_t n = recv(fd, buf, 65535, 0);",
            "    if (n < 0) n = 0;",
            "    buf[n] = 0;",
            "    return buf;",
            "}",
            "// epoll event-loop support. A single global epoll fd + a ready-fd",
            "// ring buffer, so xlang treats epoll_wait(timeout) as \"next ready fd\".",
            "#define __XLANG_EPQ_CAP 8192",
            "static int32_t __xlang_epfd_g = -1;",
            "static int __xlang_epq_fd[__XLANG_EPQ_CAP];",
            "static int __xlang_epq_head = 0;",
            "static int __xlang_epq_tail = 0;",
            "int32_t __xlang_epoll_create() {",
            "    __xlang_epfd_g = epoll_create1(0);",
            "    return __xlang_epfd_g;",
            "}",
            "int32_t __xlang_epoll_add(int32_t fd) {",
            "    struct epoll_event ev;",
            "    ev.events = EPOLLIN;",
            "    ev.data.fd = fd;",
            "    return epoll_ctl(__xlang_epfd_g, EPOLL_CTL_ADD, fd, &ev) == 0 ? 0 : -1;",
            "}",
            "int32_t __xlang_epoll_del(int32_t fd) {",
            "    epoll_ctl(__xlang_epfd_g, EPOLL_CTL_DEL, fd, 0);",
            "    return 0;",
            "}",
            "int32_t __xlang_epoll_wait(int32_t timeout) {",
            "    if (__xlang_epq_head != __xlang_epq_tail) {",
            "        int fd = __xlang_epq_fd[__xlang_epq_head];",
            "        __xlang_epq_head = (__xlang_epq_head + 1) % __XLANG_EPQ_CAP;",
            "        return (int32_t)fd;",
            "    }",
            "    struct epoll_event events[256];",
            "    int n = epoll_wait(__xlang_epfd_g, events, 256, timeout);",
            "    if (n <= 0) return -1;",
            "    int i;",
            "    for (i = 0; i < n; i++) {",
            "        __xlang_epq_fd[__xlang_epq_tail] = events[i].data.fd;",
            "        __xlang_epq_tail = (__xlang_epq_tail + 1) % __XLANG_EPQ_CAP;",
            "    }",
            "    int fd = __xlang_epq_fd[__xlang_epq_head];",
            "    __xlang_epq_head = (__xlang_epq_head + 1) % __XLANG_EPQ_CAP;",
            "    return (int32_t)fd;",
            "}",
            "int32_t __xlang_set_nonblock(int32_t fd) {",
            "    int flags = fcntl(fd, F_GETFL, 0);",
            "    return fcntl(fd, F_SETFL, flags | O_NONBLOCK) == 0 ? 0 : -1;",
            "}",
            "int32_t __xlang_set_nodelay(int32_t fd) {",
            "    int flag = 1;",
            "    return setsockopt(fd, IPPROTO_TCP, TCP_NODELAY, &flag, sizeof(flag)) == 0 ? 0 : -1;",
            "}",
            "int32_t __xlang_open_read(const char* path) {",
            "    return (int32_t)open(path, O_RDONLY);",
            "}",
            "int32_t __xlang_open_write(const char* path) {",
            "    return (int32_t)open(path, O_WRONLY | O_CREAT | O_TRUNC, 0644);",
            "}",
            "int32_t __xlang_open_append(const char* path) {",
            "    return (int32_t)open(path, O_WRONLY | O_CREAT | O_APPEND, 0644);",
            "}",
            "// Process control for the shell: pipe(2) ends in globals (one pipeline",
            "// at a time — the shell waits on each line before reading the next).",
            "static int32_t __xlang_pipe_r = -1;",
            "static int32_t __xlang_pipe_w = -1;",
            "int32_t __xlang_make_pipe() {",
            "    int p[2];",
            "    if (pipe(p) != 0) return -1;",
            "    __xlang_pipe_r = p[0];",
            "    __xlang_pipe_w = p[1];",
            "    return 0;",
            "}",
            "int32_t __xlang_pipe_read_end() { return __xlang_pipe_r; }",
            "// Indexed pipe pool: supports N-stage pipelines (up to 17 stages).",
            "#define __XLANG_PIPE_POOL 16",
            "static int32_t __xlang_pr[__XLANG_PIPE_POOL];",
            "static int32_t __xlang_pw[__XLANG_PIPE_POOL];",
            "int32_t __xlang_make_pipe_at(int32_t idx) {",
            "    int p[2];",
            "    if (idx < 0 || idx >= __XLANG_PIPE_POOL) return -1;",
            "    if (pipe(p) != 0) return -1;",
            "    __xlang_pr[idx] = p[0];",
            "    __xlang_pw[idx] = p[1];",
            "    return 0;",
            "}",
            "int32_t __xlang_pipe_r_at(int32_t idx) {",
            "    return (idx >= 0 && idx < __XLANG_PIPE_POOL) ? __xlang_pr[idx] : -1;",
            "}",
            "int32_t __xlang_pipe_w_at(int32_t idx) {",
            "    return (idx >= 0 && idx < __XLANG_PIPE_POOL) ? __xlang_pw[idx] : -1;",
            "}",
            "int32_t __xlang_pipe_write_end() { return __xlang_pipe_w; }",
            "int32_t __xlang_dup2(int32_t oldfd, int32_t newfd) {",
            "    return dup2(oldfd, newfd) < 0 ? -1 : 0;",
            "}",
            "int32_t __xlang_exec_sh(const char* cmd) {",
            "    execl(\"/bin/sh\", \"sh\", \"-c\", cmd, (char*)NULL);",
            "    return -1;",
            "}",
            "// Tokenize cmd by whitespace and execvp(argv[0], argv) — PATH-based, so a",
            "// shell with PATH=xlang-bin runs ONLY xlang coreutils (a pure xlang",
            "// userland). Returns -1 only if exec fails (child should then exit).",
            "int32_t __xlang_exec_split(const char* cmd) {",
            "    char buf[4096];",
            "    strncpy(buf, cmd, 4095); buf[4095] = 0;",
            "    char* argv[128];",
            "    int ac = 0;",
            "    char* p = buf;",
            "    while (*p) {",
            "        while (*p == ' ' || *p == '\\t') p++;",
            "        if (!*p) break;",
            "        if (ac >= 127) break;",
            "        argv[ac++] = p;",
            "        while (*p && *p != ' ' && *p != '\\t') p++;",
            "        if (*p) { *p = 0; p++; }",
            "    }",
            "    argv[ac] = (char*)NULL;",
            "    if (ac == 0) return -1;",
            "    execvp(argv[0], argv);",
            "    return -1;",
            "}",
            "int32_t __xlang_wait_child() {",
            "    int st = 0;",
            "    pid_t p = wait(&st);",
            "    return (int32_t)p;",
            "}",
            "int32_t __xlang_wait_status() {",
            "    int st = 0;",
            "    wait(&st);",
            "    if (WIFEXITED(st)) return WEXITSTATUS(st);",
            "    return 1;",
            "}",
            "char* __xlang_read_fd(int32_t fd) {",
            "    size_t cap = 65536, len = 0;",
            "    char* buf = (char*)malloc(cap);",
            "    ssize_t r;",
            "    while ((r = read(fd, buf + len, cap - len)) > 0) {",
            "        len += (size_t)r;",
            "        if (len + 1 >= cap) { cap *= 2; buf = (char*)realloc(buf, cap); }",
            "    }",
            "    buf[len] = 0;",
            "    return buf;",
            "}",
            "int32_t __xlang_setenv(const char* name, const char* value) {",
            "    return setenv(name, value, 1) == 0 ? 0 : -1;",
            "}",
            "// File fd cache: hot files keep their fd open + size known, so a request",
            "// skips open/fstat/close (what nginx does). Simple linear map, cap 512.",
            "#define __XLANG_FC_N 512",
            "static char* __xlang_fc_path[__XLANG_FC_N];",
            "static int __xlang_fc_fd[__XLANG_FC_N];",
            "static int32_t __xlang_fc_size[__XLANG_FC_N];",
            "static int __xlang_fc_len = 0;",
            "int32_t __xlang_cache_open(const char* path) {",
            "    int i;",
            "    for (i = 0; i < __xlang_fc_len; i++) {",
            "        if (strcmp(__xlang_fc_path[i], path) == 0) return (int32_t)__xlang_fc_fd[i];",
            "    }",
            "    if (__xlang_fc_len >= __XLANG_FC_N) return -1;",
            "    int fd = open(path, O_RDONLY);",
            "    if (fd < 0) return -1;",
            "    struct stat st;",
            "    if (fstat(fd, &st) != 0) { close(fd); return -1; }",
            "    __xlang_fc_path[__xlang_fc_len] = strdup(path);",
            "    __xlang_fc_fd[__xlang_fc_len] = fd;",
            "    __xlang_fc_size[__xlang_fc_len] = (int32_t)st.st_size;",
            "    __xlang_fc_len++;",
            "    return (int32_t)fd;",
            "}",
            "int32_t __xlang_cache_size(const char* path) {",
            "    int i;",
            "    for (i = 0; i < __xlang_fc_len; i++) {",
            "        if (strcmp(__xlang_fc_path[i], path) == 0) return __xlang_fc_size[i];",
            "    }",
            "    return -1;",
            "}",
            "int32_t __xlang_sendfile_fd(int32_t out_fd, int32_t in_fd, int32_t len) {",
            "    off_t off = 0;",
            "    size_t remaining = (size_t)len;",
            "    while (remaining > 0) {",
            "        ssize_t s = sendfile(out_fd, in_fd, &off, remaining);",
            "        if (s > 0) { remaining -= (size_t)s; continue; }",
            "        // non-blocking socket buffer full: retry when writable. This keeps",
            "        // the send complete (no truncation) on non-blocking sockets while",
            "        // staying out of the way for small bodies that never hit EAGAIN.",
            "        if (s < 0 && (errno == EAGAIN || errno == EWOULDBLOCK)) { sched_yield(); continue; }",
            "        break;",
            "    }",
            "    return (int32_t)((size_t)len - remaining);",
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
            "int32_t __xlang_is_dir(const char* path) {",
            "    struct stat st;",
            "    if (stat(path, &st) != 0) return 0;",
            "    return S_ISDIR(st.st_mode) ? 1 : 0;",
            "}",
            "int32_t __xlang_file_size(const char* path) {",
            "    struct stat st;",
            "    if (stat(path, &st) != 0) return 0;",
            "    return (int32_t)st.st_size;",
            "}",
            "int32_t __xlang_file_exists(const char* path) {",
            "    struct stat st;",
            "    return stat(path, &st) == 0 ? 1 : 0;",
            "}",
            "char* __xlang_getcwd() {",
            "    char* buf = (char*)malloc(4096);",
            "    return getcwd(buf, 4096);",
            "}",
            "char* __xlang_readlink(const char* path) {",
            "    char* buf = (char*)malloc(4096);",
            "    ssize_t n = readlink(path, buf, 4095);",
            "    if (n < 0) { buf[0] = 0; return buf; }",
            "    buf[n] = 0;",
            "    return buf;",
            "}",
            "char* __xlang_realpath(const char* path) {",
            "    char* resolved = realpath(path, NULL);",
            "    return resolved ? resolved : \"\";",
            "}",
            "extern char** environ;",
            "int32_t __xlang_env_count() {",
            "    int32_t n = 0;",
            "    while (environ[n]) n++;",
            "    return n;",
            "}",
            "const char* __xlang_env_entry(int32_t idx) {",
            "    extern char** environ;",
            "    int32_t n = 0;",
            "    while (environ[n]) {",
            "        if (n == idx) return environ[n];",
            "        n++;",
            "    }",
            "    return \"\";",
            "}",
            "const char* __xlang_tty() {",
            "    char* name = ttyname(0);",
            "    return name ? name : \"\";",
            "}",
            "const char* __xlang_uname_machine() {",
            "    struct utsname u;",
            "    if (uname(&u) != 0) return \"\";",
            "    char* m = (char*)malloc(strlen(u.machine) + 1);",
            "    strcpy(m, u.machine);",
            "    return m;",
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
            "sha256_hex" => format!("__xlang_sha256_hex({a})"),
            "md5_hex" => format!("__xlang_md5_hex({a})"),
            "sha1_hex" => format!("__xlang_sha1_hex({a})"),
            "sha512_hex" => format!("__xlang_sha512_hex({a})"),
            "sha224_hex" => format!("__xlang_sha224_hex({a})"),
            "sha384_hex" => format!("__xlang_sha384_hex({a})"),
            "pad_int" => {
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                format!("__xlang_pad_int({a}, {b})")
            }
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
            "str_trim" => format!("__xlang_str_trim({a})"),
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
            "is_dir" => format!("__xlang_is_dir({a})"),
            "file_size" => format!("__xlang_file_size({a})"),
            "file_exists" => format!("__xlang_file_exists({a})"),
            "chdir" => format!("chdir(({a}))"),
            "make_dir" => format!("mkdir({a}, 0755)"),
            "kill" => {
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                format!("kill(({a}), ({b}))")
            }
            "random_int" => format!("(int32_t)(rand() % ({a}))"),
            "sb_push" => format!("__xlang_sb_push({a})"),
            "sb_push_char" => format!("__xlang_sb_push_char({a})"),
            "getenv" => format!("(getenv({a}) ? getenv({a}) : \"\")"),
            "setenv" => {
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                format!("__xlang_setenv({a}, {b})")
            }
            "readlink" => format!("__xlang_readlink({a})"),
            "realpath" => format!("__xlang_realpath({a})"),
            "env_entry" => format!("__xlang_env_entry({a})"),
            "link_file" => {
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                format!("link(({a}), ({b}))")
            }
            "truncate_file" => {
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                format!("truncate(({a}), ({b}))")
            }
            "mkfifo" => format!("mkfifo(({a}), 0644)"),
            "rmdir" => format!("rmdir(({a}))"),
            "str_to_int_oct" => format!("(int32_t)strtol({a}, 0, 8)"),
            "chmod" => {
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                format!("chmod(({a}), ({b}))")
            }
            "symlink" => {
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                format!("symlink(({a}), ({b}))")
            }
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
            "system" => format!("system({a})"),
            "sleep_sec" => format!("(unsigned)sleep(({a}))"),
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
            "epoll_add" => format!("__xlang_epoll_add({a})"),
            "epoll_del" => format!("__xlang_epoll_del({a})"),
            "epoll_wait" => format!("__xlang_epoll_wait({a})"),
            "set_nonblock" => format!("__xlang_set_nonblock({a})"),
            "set_nodelay" => format!("__xlang_set_nodelay({a})"),
            "open_read" => format!("__xlang_open_read({a})"),
            "read_fd" => format!("__xlang_read_fd({a})"),
            "open_write" => format!("__xlang_open_write({a})"),
            "open_append" => format!("__xlang_open_append({a})"),
            "make_pipe_at" => format!("__xlang_make_pipe_at({a})"),
            "pipe_r_at" => format!("__xlang_pipe_r_at({a})"),
            "pipe_w_at" => format!("__xlang_pipe_w_at({a})"),
            "exec_sh" => format!("__xlang_exec_sh({a})"),
            "exec_split" => format!("__xlang_exec_split({a})"),
            "dup2" => {
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                format!("__xlang_dup2({a}, {b})")
            }
            "cache_open" => format!("__xlang_cache_open({a})"),
            "cache_size" => format!("__xlang_cache_size({a})"),
            "sendfile_fd" => {
                let (Some(second), Some(third)) = (args.get(1), args.get(2)) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                let c = self.gen_expr(third)?;
                format!("__xlang_sendfile_fd({a}, {b}, {c})")
            }
            "send_str" => {
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                format!("send({a}, {b}, strlen({b}), 0)")
            }
            "send_bytes" => {
                let (Some(second), Some(third)) = (args.get(1), args.get(2)) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                let c = self.gen_expr(third)?;
                format!("send({a}, {b}, (size_t)({c}), 0)")
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
            "make_pipe" => "__xlang_make_pipe()".to_string(),
            "pipe_read_end" => "__xlang_pipe_read_end()".to_string(),
            "pipe_write_end" => "__xlang_pipe_write_end()".to_string(),
            "wait_child" => "__xlang_wait_child()".to_string(),
            "wait_status" => "__xlang_wait_status()".to_string(),
            "epoll_create" => "__xlang_epoll_create()".to_string(),
            "argc" => "(__xlang_argc_g)".to_string(),
            "read_stdin" => "__xlang_read_stdin()".to_string(),
            "read_line" => "__xlang_read_line()".to_string(),
            "sb_new" => "__xlang_sb_new()".to_string(),
            "sb_str" => "__xlang_sb_str()".to_string(),
            "time_str" => "__xlang_time_str()".to_string(),
            "random_seed" => "srand((unsigned)time(NULL))".to_string(),
            "getcwd" => "__xlang_getcwd()".to_string(),
            "env_count" => "__xlang_env_count()".to_string(),
            "tty" => "__xlang_tty()".to_string(),
            "uname_machine" => "__xlang_uname_machine()".to_string(),
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
            Expr::StringLiteral { value } => {
                Ok(serde_json::to_string(value)?.replace("\\u001b", "\\x1b"))
            }
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
