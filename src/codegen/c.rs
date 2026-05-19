use crate::ast::*;
use crate::error::{XError, XResult};
use std::collections::{BTreeMap, HashMap};

#[derive(Default)]
pub struct CGen {
    lines: Vec<String>,
    indent: usize,
    scopes: Vec<HashMap<String, TypeNode>>,
    temp_counter: usize,
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
            if let Item::StructDecl { .. } = item {
                self.gen_struct(item)?;
                self.emit("");
            }
        }

        for item in &program.items {
            match item {
                Item::FnDecl { .. } => {
                    self.gen_fn(item)?;
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
            match item {
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
            match stmt {
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
        stmt: &Stmt,
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
        for stmt in &body.statements {
            self.gen_stmt(stmt)?;
        }
        self.pop_scope();
        self.indent -= 1;
        self.emit("}");
        Ok(())
    }

    fn gen_stmt(&mut self, stmt: &Stmt) -> XResult<()> {
        match stmt {
            Stmt::LetStmt {
                name, ty, value, ..
            } => {
                self.emit(&format!(
                    "{} {} = {};",
                    self.c_type(ty)?,
                    name,
                    self.gen_expr(value)?
                ));
                self.declare_var(name, ty.clone());
            }
            Stmt::ReturnStmt { value } => match value {
                Some(expr) => self.emit(&format!("return {};", self.gen_expr(expr)?)),
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
            Stmt::MatchStmt { .. } => {
                return Err(XError::Codegen(format!(
                    "C backend does not support statement yet: {:?}",
                    stmt_kind(stmt)
                )));
            }
        }
        Ok(())
    }

    fn gen_for_stmt(&mut self, iterator: &str, iterable: &Expr, body: &Block) -> XResult<()> {
        let Expr::Identifier {
            name: iterable_name,
        } = iterable
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

    fn gen_expr(&self, expr: &Expr) -> XResult<String> {
        match expr {
            Expr::IntLiteral { value } | Expr::FloatLiteral { value } => Ok(value.clone()),
            Expr::StringLiteral { value } => Ok(serde_json::to_string(value)?),
            Expr::BoolLiteral { value } => Ok(if *value { "true" } else { "false" }.to_string()),
            Expr::Identifier { name } => Ok(name.clone()),
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

fn stmt_kind(stmt: &Stmt) -> &'static str {
    match stmt {
        Stmt::LetStmt { .. } => "LetStmt",
        Stmt::IfStmt { .. } => "IfStmt",
        Stmt::ForStmt { .. } => "ForStmt",
        Stmt::WhileStmt { .. } => "WhileStmt",
        Stmt::MatchStmt { .. } => "MatchStmt",
        Stmt::ReturnStmt { .. } => "ReturnStmt",
        Stmt::BreakStmt => "BreakStmt",
        Stmt::ContinueStmt => "ContinueStmt",
        Stmt::ExprStmt { .. } => "ExprStmt",
    }
}
