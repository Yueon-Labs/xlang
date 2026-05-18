use crate::ast::*;
use crate::error::{XError, XResult};
use crate::lexer::{Token, TokenKind};

pub struct Parser {
    tokens: Vec<Token>,
    i: usize,
    file: String,
}

impl Parser {
    pub(crate) fn new(tokens: Vec<Token>, file: impl Into<String>) -> Self {
        Self {
            tokens,
            i: 0,
            file: file.into(),
        }
    }

    pub fn parse(&mut self) -> XResult<Program> {
        let module = self.parse_module_decl()?;
        let mut imports = Vec::new();
        while self.check("import") {
            imports.push(self.parse_import_decl()?);
        }

        let mut items = Vec::new();
        while !self.is_eof() {
            items.push(self.parse_item()?);
        }

        Ok(Program {
            kind: "Program",
            module,
            imports,
            items,
        })
    }

    fn peek(&self) -> &Token {
        &self.tokens[self.i.min(self.tokens.len() - 1)]
    }

    fn is_eof(&self) -> bool {
        self.peek().kind == TokenKind::Eof
    }

    fn check(&self, text: &str) -> bool {
        self.peek().text == text
    }

    fn bump(&mut self) -> Token {
        let tok = self.peek().clone();
        self.i += 1;
        tok
    }

    fn match_text(&mut self, text: &str) -> bool {
        if self.check(text) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, text: &str) -> XResult<Token> {
        if self.check(text) {
            Ok(self.bump())
        } else {
            let tok = self.peek();
            Err(XError::Parse(format!(
                "expected {text:?}, got {:?} at {}:{}:{}",
                tok.text, self.file, tok.line, tok.col
            )))
        }
    }

    fn expect_ident(&mut self) -> XResult<Token> {
        let tok = self.peek();
        if tok.kind == TokenKind::Ident {
            Ok(self.bump())
        } else {
            Err(XError::Parse(format!(
                "expected identifier, got {:?} at {}:{}:{}",
                tok.text, self.file, tok.line, tok.col
            )))
        }
    }

    fn parse_module_decl(&mut self) -> XResult<ModuleDecl> {
        self.expect("module")?;
        Ok(ModuleDecl {
            kind: "ModuleDecl",
            path: self.parse_path()?,
        })
    }

    fn parse_import_decl(&mut self) -> XResult<ImportDecl> {
        self.expect("import")?;
        Ok(ImportDecl {
            kind: "ImportDecl",
            path: self.parse_path()?,
        })
    }

    fn parse_path(&mut self) -> XResult<Vec<String>> {
        let mut parts = vec![self.expect_ident()?.text];
        while self.match_text(".") {
            parts.push(self.expect_ident()?.text);
        }
        Ok(parts)
    }

    fn parse_item(&mut self) -> XResult<Item> {
        if self.check("struct") {
            self.parse_struct_decl()
        } else if self.check("type") {
            self.parse_type_alias()
        } else if self.check("fn") {
            self.parse_fn_decl()
        } else {
            let tok = self.peek();
            Err(XError::Parse(format!(
                "expected item, got {:?} at {}:{}:{}",
                tok.text, self.file, tok.line, tok.col
            )))
        }
    }

    fn parse_struct_decl(&mut self) -> XResult<Item> {
        self.expect("struct")?;
        let name = self.expect_ident()?.text;
        self.expect("{")?;
        let mut fields = Vec::new();
        while !self.check("}") {
            let field_name = self.expect_ident()?.text;
            self.expect(":")?;
            let ty = self.parse_type_expr()?;
            fields.push(FieldDecl {
                kind: "FieldDecl",
                name: field_name,
                ty,
            });
        }
        self.expect("}")?;
        Ok(Item::StructDecl { name, fields })
    }

    fn parse_type_alias(&mut self) -> XResult<Item> {
        self.expect("type")?;
        let name = self.expect_ident()?.text;
        self.expect("=")?;
        let ty = self.parse_type_expr()?;
        Ok(Item::TypeAliasDecl { name, ty })
    }

    fn parse_fn_decl(&mut self) -> XResult<Item> {
        self.expect("fn")?;
        let name = self.expect_ident()?.text;
        self.expect("(")?;
        let mut params = Vec::new();
        if !self.check(")") {
            params.push(self.parse_param()?);
            while self.match_text(",") {
                params.push(self.parse_param()?);
            }
        }
        self.expect(")")?;
        self.expect(":")?;
        let return_type = self.parse_type_expr()?;
        let body = self.parse_block()?;
        Ok(Item::FnDecl {
            name,
            params,
            return_type,
            body,
        })
    }

    fn parse_param(&mut self) -> XResult<Param> {
        let name = self.expect_ident()?.text;
        self.expect(":")?;
        let ty = self.parse_type_expr()?;
        Ok(Param {
            kind: "Param",
            name,
            ty,
        })
    }

    fn parse_type_expr(&mut self) -> XResult<TypeNode> {
        let name = self.expect_ident()?.text;
        let mut args = Vec::new();
        if self.match_text("<") {
            args.push(self.parse_type_arg()?);
            while self.match_text(",") {
                args.push(self.parse_type_arg()?);
            }
            self.expect(">")?;
        }
        Ok(TypeNode::TypeExpr { name, args })
    }

    fn parse_type_arg(&mut self) -> XResult<TypeNode> {
        if self.peek().kind == TokenKind::Int {
            return Ok(TypeNode::ConstTypeArg {
                value: self.bump().text,
            });
        }
        self.parse_type_expr()
    }

    fn parse_block(&mut self) -> XResult<Block> {
        self.expect("{")?;
        let mut statements = Vec::new();
        while !self.check("}") {
            if self.is_eof() {
                let tok = self.peek();
                return Err(XError::Parse(format!(
                    "unterminated block at {}:{}:{}",
                    self.file, tok.line, tok.col
                )));
            }
            statements.push(self.parse_stmt()?);
        }
        self.expect("}")?;
        Ok(Block {
            kind: "Block",
            statements,
        })
    }

    fn parse_stmt(&mut self) -> XResult<Stmt> {
        if self.check("let") {
            self.parse_let_stmt()
        } else if self.check("if") {
            self.parse_if_stmt()
        } else if self.check("for") {
            self.parse_for_stmt()
        } else if self.check("while") {
            self.parse_while_stmt()
        } else if self.check("match") {
            self.parse_match_stmt()
        } else if self.check("return") {
            self.parse_return_stmt()
        } else if self.check("break") {
            self.expect("break")?;
            Ok(Stmt::BreakStmt)
        } else if self.check("continue") {
            self.expect("continue")?;
            Ok(Stmt::ContinueStmt)
        } else {
            Ok(Stmt::ExprStmt {
                expr: self.parse_expr()?,
            })
        }
    }

    fn parse_let_stmt(&mut self) -> XResult<Stmt> {
        self.expect("let")?;
        let mutable = self.match_text("mut");
        let name = self.expect_ident()?.text;
        self.expect(":")?;
        let ty = self.parse_type_expr()?;
        self.expect("=")?;
        let value = self.parse_expr()?;
        Ok(Stmt::LetStmt {
            mutable,
            name,
            ty,
            value,
        })
    }

    fn parse_if_stmt(&mut self) -> XResult<Stmt> {
        self.expect("if")?;
        let condition = self.parse_expr()?;
        let then_block = self.parse_block()?;
        let else_branch = if self.match_text("else") {
            if self.check("if") {
                Some(ElseBranch::IfStmt(Box::new(self.parse_if_stmt()?)))
            } else {
                Some(ElseBranch::Block(self.parse_block()?))
            }
        } else {
            None
        };
        Ok(Stmt::IfStmt {
            condition,
            then_block,
            else_branch,
        })
    }

    fn parse_for_stmt(&mut self) -> XResult<Stmt> {
        self.expect("for")?;
        let iterator = self.expect_ident()?.text;
        self.expect("in")?;
        let iterable = self.parse_expr()?;
        let body = self.parse_block()?;
        Ok(Stmt::ForStmt {
            iterator,
            iterable,
            body,
        })
    }

    fn parse_while_stmt(&mut self) -> XResult<Stmt> {
        self.expect("while")?;
        let condition = self.parse_expr()?;
        let body = self.parse_block()?;
        Ok(Stmt::WhileStmt { condition, body })
    }

    fn parse_match_stmt(&mut self) -> XResult<Stmt> {
        self.expect("match")?;
        let value = self.parse_expr()?;
        self.expect("{")?;
        let mut arms = Vec::new();
        while !self.check("}") {
            let pattern = self.parse_pattern()?;
            self.expect("=>")?;
            let body = self.parse_block()?;
            arms.push(MatchArm {
                kind: "MatchArm",
                pattern,
                body,
            });
        }
        self.expect("}")?;
        Ok(Stmt::MatchStmt { value, arms })
    }

    fn parse_pattern(&mut self) -> XResult<Pattern> {
        let name = self.expect_ident()?.text;
        let mut bindings = Vec::new();
        if self.match_text("(") {
            if !self.check(")") {
                bindings.push(self.expect_ident()?.text);
                while self.match_text(",") {
                    bindings.push(self.expect_ident()?.text);
                }
            }
            self.expect(")")?;
        }
        Ok(Pattern::VariantPattern { name, bindings })
    }

    fn parse_return_stmt(&mut self) -> XResult<Stmt> {
        self.expect("return")?;
        let value = if self.check("}") {
            None
        } else {
            Some(self.parse_expr()?)
        };
        Ok(Stmt::ReturnStmt { value })
    }

    fn parse_expr(&mut self) -> XResult<Expr> {
        self.parse_assignment()
    }

    fn parse_assignment(&mut self) -> XResult<Expr> {
        let expr = self.parse_logical_or()?;
        if self.match_text("=") {
            let value = self.parse_assignment()?;
            return Ok(Expr::AssignmentExpr {
                target: Box::new(expr),
                value: Box::new(value),
            });
        }
        Ok(expr)
    }

    fn parse_logical_or(&mut self) -> XResult<Expr> {
        let mut expr = self.parse_logical_and()?;
        while self.match_text("||") {
            let right = self.parse_logical_and()?;
            expr = binary("||", expr, right);
        }
        Ok(expr)
    }

    fn parse_logical_and(&mut self) -> XResult<Expr> {
        let mut expr = self.parse_equality()?;
        while self.match_text("&&") {
            let right = self.parse_equality()?;
            expr = binary("&&", expr, right);
        }
        Ok(expr)
    }

    fn parse_equality(&mut self) -> XResult<Expr> {
        let mut expr = self.parse_comparison()?;
        while self.check("==") || self.check("!=") {
            let op = self.bump().text;
            let right = self.parse_comparison()?;
            expr = binary(op, expr, right);
        }
        Ok(expr)
    }

    fn parse_comparison(&mut self) -> XResult<Expr> {
        let mut expr = self.parse_term()?;
        while matches!(self.peek().text.as_str(), ">" | ">=" | "<" | "<=") {
            let op = self.bump().text;
            let right = self.parse_term()?;
            expr = binary(op, expr, right);
        }
        Ok(expr)
    }

    fn parse_term(&mut self) -> XResult<Expr> {
        let mut expr = self.parse_factor()?;
        while self.check("+") || self.check("-") {
            let op = self.bump().text;
            let right = self.parse_factor()?;
            expr = binary(op, expr, right);
        }
        Ok(expr)
    }

    fn parse_factor(&mut self) -> XResult<Expr> {
        let mut expr = self.parse_unary()?;
        while self.check("*") || self.check("/") || self.check("%") {
            let op = self.bump().text;
            let right = self.parse_unary()?;
            expr = binary(op, expr, right);
        }
        Ok(expr)
    }

    fn parse_unary(&mut self) -> XResult<Expr> {
        if self.check("!") || self.check("-") {
            let op = self.bump().text;
            let value = self.parse_unary()?;
            return Ok(Expr::UnaryExpr {
                op,
                value: Box::new(value),
            });
        }
        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> XResult<Expr> {
        let mut expr = self.parse_primary()?;
        loop {
            if self.match_text("(") {
                let mut args = Vec::new();
                if !self.check(")") {
                    args.push(self.parse_expr()?);
                    while self.match_text(",") {
                        args.push(self.parse_expr()?);
                    }
                }
                self.expect(")")?;
                expr = Expr::CallExpr {
                    callee: Box::new(expr),
                    args,
                };
            } else if self.match_text(".") {
                let field = self.expect_ident()?.text;
                expr = Expr::FieldAccessExpr {
                    object: Box::new(expr),
                    field,
                };
            } else {
                return Ok(expr);
            }
        }
    }

    fn parse_primary(&mut self) -> XResult<Expr> {
        let tok = self.peek().clone();
        match tok.kind {
            TokenKind::Int => {
                self.bump();
                Ok(Expr::IntLiteral { value: tok.text })
            }
            TokenKind::Float => {
                self.bump();
                Ok(Expr::FloatLiteral { value: tok.text })
            }
            TokenKind::String => {
                self.bump();
                Ok(Expr::StringLiteral { value: tok.text })
            }
            TokenKind::Ident => {
                self.bump();
                Ok(Expr::Identifier { name: tok.text })
            }
            TokenKind::Keyword if tok.text == "true" || tok.text == "false" => {
                self.bump();
                Ok(Expr::BoolLiteral {
                    value: tok.text == "true",
                })
            }
            _ if tok.text == "(" => {
                self.expect("(")?;
                let expr = self.parse_expr()?;
                self.expect(")")?;
                Ok(expr)
            }
            _ => Err(XError::Parse(format!(
                "expected expression, got {:?} at {}:{}:{}",
                tok.text, self.file, tok.line, tok.col
            ))),
        }
    }
}

fn binary(op: impl Into<String>, left: Expr, right: Expr) -> Expr {
    Expr::BinaryExpr {
        op: op.into(),
        left: Box::new(left),
        right: Box::new(right),
    }
}
