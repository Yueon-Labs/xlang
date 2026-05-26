use crate::ast::*;
use crate::error::{XError, XResult};
use std::collections::HashMap;

#[derive(Clone, Copy)]
struct VarInfo {
    mutable: bool,
}

#[derive(Default)]
struct Checker {
    scopes: Vec<HashMap<String, VarInfo>>,
}

pub fn check_program(program: &Program) -> XResult<()> {
    let mut checker = Checker::default();
    checker.check_program(program)
}

impl Checker {
    fn check_program(&mut self, program: &Program) -> XResult<()> {
        for item in &program.items {
            if let Item::FnDecl { params, body, .. } = item {
                self.push_scope();
                for param in params {
                    self.declare(&param.name, false);
                }
                self.check_statements(&body.statements)?;
                self.pop_scope();
            }
        }
        Ok(())
    }

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn declare(&mut self, name: &str, mutable: bool) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string(), VarInfo { mutable });
        }
    }

    fn lookup(&self, name: &str) -> Option<VarInfo> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).copied())
    }

    fn check_block(&mut self, block: &Block) -> XResult<()> {
        self.push_scope();
        self.check_statements(&block.statements)?;
        self.pop_scope();
        Ok(())
    }

    fn check_statements(&mut self, statements: &[Stmt]) -> XResult<()> {
        for stmt in statements {
            self.check_stmt(stmt)?;
        }
        Ok(())
    }

    fn check_stmt(&mut self, stmt: &Stmt) -> XResult<()> {
        match stmt {
            Stmt::LetStmt {
                mutable,
                name,
                value,
                ..
            } => {
                self.check_expr(value)?;
                self.declare(name, *mutable);
            }
            Stmt::IfStmt {
                condition,
                then_block,
                else_branch,
            } => {
                self.check_expr(condition)?;
                self.check_block(then_block)?;
                match else_branch {
                    Some(ElseBranch::Block(block)) => self.check_block(block)?,
                    Some(ElseBranch::IfStmt(stmt)) => self.check_stmt(stmt)?,
                    None => {}
                }
            }
            Stmt::ForStmt {
                iterator,
                iterable,
                body,
            } => {
                self.check_expr(iterable)?;
                self.push_scope();
                self.declare(iterator, false);
                self.check_statements(&body.statements)?;
                self.pop_scope();
            }
            Stmt::WhileStmt { condition, body } => {
                self.check_expr(condition)?;
                self.check_block(body)?;
            }
            Stmt::MatchStmt { value, arms } => {
                self.check_expr(value)?;
                for arm in arms {
                    self.push_scope();
                    let Pattern::VariantPattern { bindings, .. } = &arm.pattern;
                    for binding in bindings {
                        self.declare(binding, false);
                    }
                    self.check_statements(&arm.body.statements)?;
                    self.pop_scope();
                }
            }
            Stmt::ReturnStmt { value } => {
                if let Some(value) = value {
                    self.check_expr(value)?;
                }
            }
            Stmt::ExprStmt { expr } => self.check_expr(expr)?,
            Stmt::BreakStmt | Stmt::ContinueStmt => {}
        }
        Ok(())
    }

    fn check_expr(&mut self, expr: &Expr) -> XResult<()> {
        match expr {
            Expr::IntLiteral { .. }
            | Expr::FloatLiteral { .. }
            | Expr::StringLiteral { .. }
            | Expr::BoolLiteral { .. }
            | Expr::Identifier { .. } => {}
            Expr::ArrayLiteral { elements } => {
                for element in elements {
                    self.check_expr(element)?;
                }
            }
            Expr::BinaryExpr { left, right, .. } => {
                self.check_expr(left)?;
                self.check_expr(right)?;
            }
            Expr::UnaryExpr { value, .. } => self.check_expr(value)?,
            Expr::AssignmentExpr { target, value } => {
                self.check_assignment_target(target)?;
                self.check_expr(value)?;
            }
            Expr::CallExpr { callee, args } => {
                self.check_expr(callee)?;
                for arg in args {
                    self.check_expr(arg)?;
                }
            }
            Expr::FieldAccessExpr { object, .. } => self.check_expr(object)?,
        }
        Ok(())
    }

    fn check_assignment_target(&mut self, target: &Expr) -> XResult<()> {
        match target {
            Expr::Identifier { name } => match self.lookup(name) {
                Some(VarInfo { mutable: true }) => Ok(()),
                Some(VarInfo { mutable: false }) => Err(XError::Type(format!(
                    "cannot assign to immutable variable {name:?}; declare it with `let mut {name}` if reassignment is intended"
                ))),
                None => Err(XError::Type(format!(
                    "cannot assign to unknown variable {name:?}"
                ))),
            },
            Expr::FieldAccessExpr { object, .. } => self.check_assignment_target(object),
            _ => Err(XError::Type(
                "assignment target must be a variable or field access".to_string(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::check_program;
    use crate::lexer::Lexer;
    use crate::parser::Parser;

    fn check_source(source: &str) -> String {
        let tokens = Lexer::new(source).tokenize().expect("lex source");
        let program = Parser::new(tokens, "<test>").parse().expect("parse source");
        match check_program(&program) {
            Ok(()) => "ok".to_string(),
            Err(err) => err.to_string(),
        }
    }

    #[test]
    fn rejects_assignment_to_immutable_local() {
        let err = check_source(
            r#"
module main

fn main(): i32 {
    let x: i32 = 1
    x = 2
    return x
}
"#,
        );

        assert!(err.contains("cannot assign to immutable variable \"x\""));
    }

    #[test]
    fn allows_assignment_to_mutable_local() {
        let result = check_source(
            r#"
module main

fn main(): i32 {
    let mut x: i32 = 1
    x = 2
    return x
}
"#,
        );

        assert_eq!(result, "ok");
    }

    #[test]
    fn rejects_assignment_to_function_param() {
        let err = check_source(
            r#"
module main

fn bump(x: i32): i32 {
    x = x + 1
    return x
}
"#,
        );

        assert!(err.contains("cannot assign to immutable variable \"x\""));
    }
}
