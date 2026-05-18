use crate::ast::*;
use crate::error::{XError, XResult};

#[derive(Default)]
pub struct CGen {
    lines: Vec<String>,
    indent: usize,
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

    fn c_type(&self, ty: &TypeNode) -> XResult<&'static str> {
        match ty {
            TypeNode::TypeExpr { name, args } if args.is_empty() => match name.as_str() {
                "i32" => Ok("int32_t"),
                "i64" => Ok("int64_t"),
                "f32" => Ok("float"),
                "f64" => Ok("double"),
                "bool" => Ok("bool"),
                "String" | "Str" => Ok("const char *"),
                other => Err(XError::Codegen(format!(
                    "C backend does not support type yet: {other}"
                ))),
            },
            TypeNode::TypeExpr { name, .. } => Err(XError::Codegen(format!(
                "C backend does not support generic type yet: {name}<...>"
            ))),
            TypeNode::ConstTypeArg { value } => Err(XError::Codegen(format!(
                "unexpected const type argument in C type position: {value}"
            ))),
        }
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
        for stmt in &body.statements {
            self.gen_stmt(stmt)?;
        }
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
                for inner in &body.statements {
                    self.gen_stmt(inner)?;
                }
                self.indent -= 1;
                self.emit("}");
            }
            Stmt::ExprStmt { expr } => self.emit(&format!("{};", self.gen_expr(expr)?)),
            Stmt::BreakStmt => self.emit("break;"),
            Stmt::ContinueStmt => self.emit("continue;"),
            Stmt::ForStmt { .. } | Stmt::MatchStmt { .. } => {
                return Err(XError::Codegen(format!(
                    "C backend does not support statement yet: {:?}",
                    stmt_kind(stmt)
                )));
            }
        }
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
