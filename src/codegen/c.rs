use crate::ast::*;
use crate::error::{XError, XResult};
use crate::source::Spanned;
use std::collections::{BTreeMap, HashMap};

#[derive(Default)]
pub struct CGen {
    lines: Vec<String>,
    indent: usize,
    scopes: Vec<HashMap<String, TypeNode>>,
    temp_counter: usize,
    /// Return type of the function currently being generated (for constructing
    /// Some/None/Ok/Err in `return` position).
    fn_return: Option<TypeNode>,
}

impl CGen {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn generate(mut self, program: &Program) -> XResult<String> {
        self.emit("#include <stdint.h>");
        self.emit("#include <stdbool.h>");
        self.emit("#include <stddef.h>");
        self.emit("");

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
        let params_text = if params.is_empty() {
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
        if name != "Slice" || args.len() != 1 {
            return Err(XError::Codegen(format!(
                "C backend only supports for-in over Slice<T>, got {name}<...>"
            )));
        }
        let elem_ty = args[0].clone();
        let elem_c_type = self.c_type(&elem_ty)?;
        let iter_c = self.gen_expr(iterable)?;
        let index = self.next_temp("i");

        self.emit(&format!(
            "for (size_t {index} = 0; {index} < {iter_c}.len; {index}++) {{"
        ));
        self.indent += 1;
        self.push_scope();
        self.declare_var(iterator, elem_ty);
        self.emit(&format!(
            "{elem_c_type} {iterator} = {iter_c}.data[{index}];"
        ));
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
                let mut parts = Vec::new();
                for arg in args {
                    parts.push(self.gen_expr(arg)?);
                }
                Ok(format!("{}({})", self.gen_expr(callee)?, parts.join(", ")))
            }
            Expr::FieldAccessExpr { object, field } => {
                Ok(format!("{}.{}", self.gen_expr(object)?, field))
            }
        }
    }
}
