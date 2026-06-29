use crate::ast::*;
use crate::error::{Diagnostic, Diagnostics, ErrorCode};
use crate::source::{Span, Spanned};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq)]
enum CheckedType {
    Unknown,
    IntLiteral,
    FloatLiteral,
    StringLiteral,
    Named {
        name: String,
        args: Vec<CheckedType>,
    },
    Const(String),
    ArrayLiteral(Vec<CheckedType>),
}

#[derive(Clone, Debug)]
struct VarInfo {
    mutable: bool,
    ty: CheckedType,
}

#[derive(Clone, Debug)]
struct FnSig {
    params: Vec<CheckedType>,
    return_type: CheckedType,
}

/// Type checker that accumulates structured diagnostics instead of bailing at
/// the first error. On an error it emits a `Diagnostic` and continues using
/// `CheckedType::Unknown` as a poison value (which downstream checks treat as
/// "anything goes"), so one mistake surfaces as one diagnostic rather than a
/// cascade.
#[derive(Default)]
struct Checker {
    scopes: Vec<HashMap<String, VarInfo>>,
    functions: HashMap<String, FnSig>,
    structs: HashMap<String, Vec<(String, CheckedType)>>,
    return_types: Vec<CheckedType>,
    diags: Diagnostics,
}

/// Type-check `program`, returning all accumulated diagnostics (empty = clean).
pub fn check_program(program: &Program) -> Diagnostics {
    let mut checker = Checker::default();
    checker.collect_functions(program);
    checker.collect_structs(program);
    checker.check_program(program);
    checker.diags
}

impl Checker {
    fn emit(&mut self, span: Span, code: ErrorCode, message: impl Into<String>) {
        self.diags.push(Diagnostic::error(code, span, message));
    }

    fn collect_functions(&mut self, program: &Program) {
        for item in &program.items {
            if let Item::FnDecl {
                name,
                params,
                return_type,
                ..
            } = &item.node
            {
                self.functions.insert(
                    name.clone(),
                    FnSig {
                        params: params
                            .iter()
                            .map(|param| type_from_node(&param.ty))
                            .collect(),
                        return_type: type_from_node(return_type),
                    },
                );
            }
        }
    }

    fn collect_structs(&mut self, program: &Program) {
        for item in &program.items {
            if let Item::StructDecl { name, fields } = &item.node {
                let field_types = fields
                    .iter()
                    .map(|f| (f.name.clone(), type_from_node(&f.ty)))
                    .collect();
                self.structs.insert(name.clone(), field_types);
            }
        }
    }

    fn check_program(&mut self, program: &Program) {
        for item in &program.items {
            if let Item::FnDecl {
                params,
                return_type,
                body,
                ..
            } = &item.node
            {
                self.push_scope();
                self.return_types.push(type_from_node(return_type));
                for param in params {
                    self.declare(&param.name, false, type_from_node(&param.ty));
                }
                self.check_statements(&body.statements);
                self.return_types.pop();
                self.pop_scope();
            }
        }
    }

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn declare(&mut self, name: &str, mutable: bool, ty: CheckedType) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string(), VarInfo { mutable, ty });
        }
    }

    fn lookup(&self, name: &str) -> Option<VarInfo> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).cloned())
    }

    fn current_return_type(&self) -> CheckedType {
        self.return_types
            .last()
            .cloned()
            .unwrap_or(CheckedType::Unknown)
    }

    fn check_block(&mut self, block: &Block) {
        self.push_scope();
        self.check_statements(&block.statements);
        self.pop_scope();
    }

    fn check_statements(&mut self, statements: &[Spanned<Stmt>]) {
        for stmt in statements {
            self.check_stmt(stmt);
        }
    }

    fn check_stmt(&mut self, stmt: &Spanned<Stmt>) {
        match &stmt.node {
            Stmt::LetStmt {
                mutable,
                name,
                ty,
                value,
            } => {
                let declared = type_from_node(ty);
                let actual = self.infer_expr(value);
                self.expect_assignable(
                    &declared,
                    &actual,
                    &format!("initializer for variable {name:?}"),
                    value.span,
                );
                self.declare(name, *mutable, declared);
            }
            Stmt::IfStmt {
                condition,
                then_block,
                else_branch,
            } => {
                let condition_ty = self.infer_expr(condition);
                self.expect_bool(&condition_ty, "if condition", condition.span);
                self.check_block(then_block);
                match else_branch {
                    Some(ElseBranch::Block(block)) => self.check_block(block),
                    Some(ElseBranch::IfStmt(stmt)) => self.check_stmt(stmt),
                    None => {}
                }
            }
            Stmt::ForStmt {
                iterator,
                iterable,
                body,
            } => {
                let iterable_ty = self.infer_expr(iterable);
                let iterator_ty = match &iterable_ty {
                    CheckedType::Named { name, args } if name == "Slice" && args.len() == 1 => {
                        args[0].clone()
                    }
                    CheckedType::Named { name, args } if name == "Array" && args.len() == 2 => {
                        args[0].clone()
                    }
                    CheckedType::Named { name, args } if name == "Vec" && args.len() == 1 => {
                        args[0].clone()
                    }
                    CheckedType::Unknown => CheckedType::Unknown,
                    other => {
                        self.emit(
                            iterable.span,
                            ErrorCode::TypeForInExpectsSlice,
                            format!(
                                "for-in expects Slice<T> or Array<T, N>, got {}",
                                other.display()
                            ),
                        );
                        CheckedType::Unknown
                    }
                };
                self.push_scope();
                self.declare(iterator, false, iterator_ty);
                self.check_statements(&body.statements);
                self.pop_scope();
            }
            Stmt::WhileStmt { condition, body } => {
                let condition_ty = self.infer_expr(condition);
                self.expect_bool(&condition_ty, "while condition", condition.span);
                self.check_block(body);
            }
            Stmt::MatchStmt { value, arms } => {
                self.infer_expr(value);
                for arm in arms {
                    self.push_scope();
                    let Pattern::VariantPattern { bindings, .. } = &arm.pattern;
                    for binding in bindings {
                        self.declare(binding, false, CheckedType::Unknown);
                    }
                    self.check_statements(&arm.body.statements);
                    self.pop_scope();
                }
            }
            Stmt::ReturnStmt { value } => {
                let expected = self.current_return_type();
                match value {
                    Some(value) => {
                        let actual = self.infer_expr(value);
                        self.expect_assignable(&expected, &actual, "return value", value.span);
                    }
                    None => {
                        self.emit(
                            stmt.span,
                            ErrorCode::TypeReturnMissingValue,
                            format!(
                                "return statement missing value for function returning {}",
                                expected.display()
                            ),
                        );
                    }
                }
            }
            Stmt::ExprStmt { expr } => {
                self.infer_expr(expr);
            }
            Stmt::BreakStmt | Stmt::ContinueStmt => {}
        }
    }

    fn infer_expr(&mut self, expr: &Spanned<Expr>) -> CheckedType {
        let span = expr.span;
        match &expr.node {
            Expr::IntLiteral { .. } => CheckedType::IntLiteral,
            Expr::FloatLiteral { .. } => CheckedType::FloatLiteral,
            Expr::StringLiteral { .. } => CheckedType::StringLiteral,
            Expr::BoolLiteral { .. } => CheckedType::named("bool"),
            Expr::Identifier { name } => {
                if is_builtin_variant(name) {
                    CheckedType::Unknown
                } else {
                    match self.lookup(name) {
                        Some(var) => var.ty,
                        None => {
                            self.emit(
                                span,
                                ErrorCode::TypeUnknownVar,
                                format!("unknown variable {name:?}"),
                            );
                            CheckedType::Unknown
                        }
                    }
                }
            }
            Expr::ArrayLiteral { elements } => {
                let mut element_types = Vec::new();
                for element in elements {
                    element_types.push(self.infer_expr(element));
                }
                CheckedType::ArrayLiteral(element_types)
            }
            Expr::BinaryExpr { op, left, right } => self.infer_binary_expr(op, left, right, span),
            Expr::UnaryExpr { op, value } => self.infer_unary_expr(op, value),
            Expr::AssignmentExpr { target, value } => {
                let target_ty = self.check_assignment_target(target);
                let value_ty = self.infer_expr(value);
                self.expect_assignable(&target_ty, &value_ty, "assignment value", value.span);
                target_ty
            }
            Expr::CallExpr { callee, args } => self.infer_call_expr(callee, args, span),
            Expr::FieldAccessExpr { object, field } => {
                let obj_ty = self.infer_expr(object);
                if let CheckedType::Named { name, .. } = &obj_ty
                    && let Some(fields) = self.structs.get(name)
                    && let Some((_, field_ty)) = fields.iter().find(|(n, _)| n == field)
                {
                    return field_ty.clone();
                }
                CheckedType::Unknown
            }
            Expr::IndexExpr { object, index } => {
                let obj_ty = self.infer_expr(object);
                let idx_ty = self.infer_expr(index);
                if !idx_ty.is_unknown() && !idx_ty.is_int_literal() && !idx_ty.is_integer_scalar() {
                    self.emit(
                        span,
                        ErrorCode::TypeMismatch,
                        format!("array index must be an integer, got {}", idx_ty.display()),
                    );
                }
                match &obj_ty {
                    CheckedType::Named { name, args } if name == "Array" && args.len() == 2 => {
                        args[0].clone()
                    }
                    CheckedType::Named { name, args } if name == "Slice" && args.len() == 1 => {
                        args[0].clone()
                    }
                    CheckedType::Named { name, args } if name == "Vec" && args.len() == 1 => {
                        args[0].clone()
                    }
                    CheckedType::Unknown => CheckedType::Unknown,
                    other => {
                        self.emit(
                            span,
                            ErrorCode::TypeMismatch,
                            format!("cannot index into {}", other.display()),
                        );
                        CheckedType::Unknown
                    }
                }
            }
            Expr::StructLiteral { name, fields } => {
                // Clone the declared fields so we don't hold a borrow of self
                // across the mutable infer_expr calls below.
                let decl_fields = self.structs.get(name).cloned();
                match decl_fields {
                    Some(decl_fields) => {
                        for f in fields {
                            let val_ty = self.infer_expr(&f.value);
                            match decl_fields.iter().find(|(n, _)| n == &f.name) {
                                Some((_, field_ty)) => self.expect_assignable(
                                    field_ty,
                                    &val_ty,
                                    &format!("struct field {:?}", f.name),
                                    f.value.span,
                                ),
                                None => self.emit(
                                    f.value.span,
                                    ErrorCode::TypeMismatch,
                                    format!("struct {name:?} has no field {:?}", f.name),
                                ),
                            }
                        }
                        CheckedType::named(name)
                    }
                    None => {
                        self.emit(
                            span,
                            ErrorCode::TypeMismatch,
                            format!("unknown struct type {name:?}"),
                        );
                        CheckedType::Unknown
                    }
                }
            }
        }
    }

    fn infer_binary_expr(
        &mut self,
        op: &str,
        left: &Spanned<Expr>,
        right: &Spanned<Expr>,
        span: Span,
    ) -> CheckedType {
        let left_ty = self.infer_expr(left);
        let right_ty = self.infer_expr(right);
        match op {
            "+" | "-" | "*" | "/" | "%" => self.infer_numeric_result(op, &left_ty, &right_ty, span),
            ">" | ">=" | "<" | "<=" => {
                self.expect_numeric_pair(op, &left_ty, &right_ty, span);
                CheckedType::named("bool")
            }
            "==" | "!=" => {
                if !left_ty.is_unknown()
                    && !right_ty.is_unknown()
                    && !self.types_compatible(&left_ty, &right_ty)
                    && !self.types_compatible(&right_ty, &left_ty)
                {
                    self.emit(
                        span,
                        ErrorCode::TypeOperatorMismatch,
                        format!(
                            "operator {op} cannot compare {} and {}",
                            left_ty.display(),
                            right_ty.display()
                        ),
                    );
                }
                CheckedType::named("bool")
            }
            "&&" | "||" => {
                self.expect_bool(&left_ty, &format!("left operand of {op}"), left.span);
                self.expect_bool(&right_ty, &format!("right operand of {op}"), right.span);
                CheckedType::named("bool")
            }
            _ => CheckedType::Unknown,
        }
    }

    fn infer_unary_expr(&mut self, op: &str, value: &Spanned<Expr>) -> CheckedType {
        let value_ty = self.infer_expr(value);
        match op {
            "!" => {
                self.expect_bool(&value_ty, "operand of !", value.span);
                CheckedType::named("bool")
            }
            "-" => {
                self.expect_numeric(&value_ty, "operand of unary -", value.span);
                value_ty
            }
            _ => CheckedType::Unknown,
        }
    }

    fn infer_call_expr(
        &mut self,
        callee: &Spanned<Expr>,
        args: &[Spanned<Expr>],
        span: Span,
    ) -> CheckedType {
        if let Expr::Identifier { name } = &callee.node {
            if let Some(sig) = self.functions.get(name).cloned() {
                if args.len() != sig.params.len() {
                    self.emit(
                        span,
                        ErrorCode::TypeArgCount,
                        format!(
                            "function {name:?} expects {} arguments, got {}",
                            sig.params.len(),
                            args.len()
                        ),
                    );
                    for arg in args {
                        self.infer_expr(arg);
                    }
                    return CheckedType::Unknown;
                }
                for (index, (arg, param_ty)) in args.iter().zip(&sig.params).enumerate() {
                    let arg_ty = self.infer_expr(arg);
                    self.expect_assignable(
                        param_ty,
                        &arg_ty,
                        &format!("argument {} for function {name:?}", index + 1),
                        arg.span,
                    );
                }
                return sig.return_type;
            }

            for arg in args {
                self.infer_expr(arg);
            }
            return CheckedType::Unknown;
        }

        self.infer_expr(callee);
        for arg in args {
            self.infer_expr(arg);
        }
        CheckedType::Unknown
    }

    fn check_assignment_target(&mut self, target: &Spanned<Expr>) -> CheckedType {
        let span = target.span;
        match &target.node {
            Expr::Identifier { name } => match self.lookup(name) {
                Some(VarInfo {
                    mutable: true, ty, ..
                }) => ty,
                Some(VarInfo { mutable: false, .. }) => {
                    self.emit(
                        span,
                        ErrorCode::TypeImmutableAssign,
                        format!(
                            "cannot assign to immutable variable {name:?}; declare it with `let mut {name}` if reassignment is intended"
                        ),
                    );
                    CheckedType::Unknown
                }
                None => {
                    self.emit(
                        span,
                        ErrorCode::TypeUnknownAssignTarget,
                        format!("cannot assign to unknown variable {name:?}"),
                    );
                    CheckedType::Unknown
                }
            },
            Expr::FieldAccessExpr { object, .. } => {
                self.check_assignment_target(object);
                CheckedType::Unknown
            }
            Expr::IndexExpr { object, .. } => {
                let obj_ty = self.infer_expr(object);
                match &obj_ty {
                    CheckedType::Named { name, args } if name == "Array" && args.len() == 2 => {
                        args[0].clone()
                    }
                    CheckedType::Unknown => CheckedType::Unknown,
                    other => {
                        self.emit(
                            span,
                            ErrorCode::TypeMismatch,
                            format!("cannot assign to index of {}", other.display()),
                        );
                        CheckedType::Unknown
                    }
                }
            }
            _ => {
                self.emit(
                    span,
                    ErrorCode::TypeAssignmentTarget,
                    "assignment target must be a variable or field access",
                );
                CheckedType::Unknown
            }
        }
    }

    fn infer_numeric_result(
        &mut self,
        op: &str,
        left: &CheckedType,
        right: &CheckedType,
        span: Span,
    ) -> CheckedType {
        self.expect_numeric_pair(op, left, right, span);
        if left.is_unknown() || right.is_unknown() {
            return CheckedType::Unknown;
        }
        if left == right {
            return left.clone();
        }
        if left.is_int_literal() && right.is_integer_scalar() {
            return right.clone();
        }
        if right.is_int_literal() && left.is_integer_scalar() {
            return left.clone();
        }
        if left.is_float_literal() && right.is_float_scalar() {
            return right.clone();
        }
        if right.is_float_literal() && left.is_float_scalar() {
            return left.clone();
        }
        if left.is_int_literal() && right.is_int_literal() {
            return CheckedType::IntLiteral;
        }
        if left.is_float_literal() && right.is_float_literal() {
            return CheckedType::FloatLiteral;
        }
        // Incompatible concrete numerics (e.g. i32 + f64): expect_numeric_pair
        // already emitted the "cannot combine" diagnostic above.
        CheckedType::Unknown
    }

    fn expect_assignable(
        &mut self,
        expected: &CheckedType,
        actual: &CheckedType,
        context: &str,
        span: Span,
    ) {
        if !self.types_compatible(expected, actual) {
            self.emit(
                span,
                ErrorCode::TypeMismatch,
                format!(
                    "{context} expects {}, got {}",
                    expected.display(),
                    actual.display()
                ),
            );
        }
    }

    fn types_compatible(&self, expected: &CheckedType, actual: &CheckedType) -> bool {
        if expected.is_unknown() || actual.is_unknown() {
            return true;
        }
        match (expected, actual) {
            (CheckedType::Named { name, args }, CheckedType::ArrayLiteral(element_types))
                if name == "Array" && args.len() == 2 =>
            {
                let Some(expected_len) = array_len(&args[1]) else {
                    return true;
                };
                if element_types.len() != expected_len {
                    return false;
                }
                element_types
                    .iter()
                    .all(|actual| self.types_compatible(&args[0], actual))
            }
            (CheckedType::Named { name, .. }, CheckedType::IntLiteral)
                if name == "i32" || name == "i64" =>
            {
                true
            }
            (CheckedType::Named { name, .. }, CheckedType::FloatLiteral)
                if name == "f32" || name == "f64" =>
            {
                true
            }
            (CheckedType::Named { name, .. }, CheckedType::StringLiteral)
                if name == "String" || name == "Str" =>
            {
                true
            }
            _ => expected == actual,
        }
    }

    fn expect_bool(&mut self, ty: &CheckedType, context: &str, span: Span) {
        if !(ty.is_unknown() || ty.is_bool()) {
            self.emit(
                span,
                ErrorCode::TypeBoolRequired,
                format!("{context} must be bool, got {}", ty.display()),
            );
        }
    }

    fn expect_numeric_pair(
        &mut self,
        op: &str,
        left: &CheckedType,
        right: &CheckedType,
        span: Span,
    ) {
        self.expect_numeric(left, &format!("left operand of {op}"), span);
        self.expect_numeric(right, &format!("right operand of {op}"), span);
        if left.is_unknown() || right.is_unknown() {
            return;
        }
        if !(self.types_compatible(left, right) || self.types_compatible(right, left)) {
            self.emit(
                span,
                ErrorCode::TypeOperatorMismatch,
                format!(
                    "operator {op} cannot combine {} and {}",
                    left.display(),
                    right.display()
                ),
            );
        }
    }

    fn expect_numeric(&mut self, ty: &CheckedType, context: &str, span: Span) {
        if !(ty.is_unknown() || ty.is_numeric()) {
            self.emit(
                span,
                ErrorCode::TypeNumericRequired,
                format!("{context} must be numeric, got {}", ty.display()),
            );
        }
    }
}

impl CheckedType {
    fn named(name: &str) -> Self {
        Self::Named {
            name: name.to_string(),
            args: Vec::new(),
        }
    }

    fn is_unknown(&self) -> bool {
        matches!(self, Self::Unknown)
    }

    fn is_bool(&self) -> bool {
        matches!(self, Self::Named { name, args } if name == "bool" && args.is_empty())
    }

    fn is_int_literal(&self) -> bool {
        matches!(self, Self::IntLiteral)
    }

    fn is_float_literal(&self) -> bool {
        matches!(self, Self::FloatLiteral)
    }

    fn is_integer_scalar(&self) -> bool {
        matches!(self, Self::Named { name, args } if args.is_empty() && matches!(name.as_str(), "i32" | "i64"))
    }

    fn is_float_scalar(&self) -> bool {
        matches!(self, Self::Named { name, args } if args.is_empty() && matches!(name.as_str(), "f32" | "f64"))
    }

    fn is_numeric(&self) -> bool {
        self.is_int_literal()
            || self.is_float_literal()
            || self.is_integer_scalar()
            || self.is_float_scalar()
    }

    fn display(&self) -> String {
        match self {
            Self::Unknown => "unknown".to_string(),
            Self::IntLiteral => "integer literal".to_string(),
            Self::FloatLiteral => "float literal".to_string(),
            Self::StringLiteral => "string literal".to_string(),
            Self::Named { name, args } if args.is_empty() => name.clone(),
            Self::Named { name, args } => {
                let args = args
                    .iter()
                    .map(Self::display)
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{name}<{args}>")
            }
            Self::Const(value) => value.clone(),
            Self::ArrayLiteral(elements) => {
                format!("array literal with {} elements", elements.len())
            }
        }
    }
}

fn type_from_node(ty: &TypeNode) -> CheckedType {
    match ty {
        TypeNode::TypeExpr { name, args } => CheckedType::Named {
            name: name.clone(),
            args: args.iter().map(type_from_node).collect(),
        },
        TypeNode::ConstTypeArg { value } => CheckedType::Const(value.clone()),
    }
}

fn array_len(ty: &CheckedType) -> Option<usize> {
    let CheckedType::Const(value) = ty else {
        return None;
    };
    value.parse().ok()
}

fn is_builtin_variant(name: &str) -> bool {
    matches!(name, "Some" | "None" | "Ok" | "Err")
}

#[cfg(test)]
mod tests {
    use super::check_program;
    use crate::error::Diagnostics;
    use crate::lexer::Lexer;
    use crate::parser::Parser;

    fn check_source(source: &str) -> Diagnostics {
        let (tokens, _lex_diags) = Lexer::new(source).tokenize();
        let program = Parser::new(tokens, "<test>").parse().expect("parse source");
        check_program(&program)
    }

    fn first_message(diags: &Diagnostics) -> &str {
        diags
            .items
            .first()
            .map(|d| d.message.as_str())
            .unwrap_or("")
    }

    #[test]
    fn rejects_assignment_to_immutable_local() {
        let diags = check_source(
            r#"
module main

fn main(): i32 {
    let x: i32 = 1
    x = 2
    return x
}
"#,
        );
        assert!(first_message(&diags).contains("cannot assign to immutable variable \"x\""));
    }

    #[test]
    fn allows_assignment_to_mutable_local() {
        let diags = check_source(
            r#"
module main

fn main(): i32 {
    let mut x: i32 = 1
    x = 2
    return x
}
"#,
        );
        assert!(
            diags.is_empty(),
            "expected no diagnostics, got: {:?}",
            diags.items
        );
    }

    #[test]
    fn rejects_assignment_to_function_param() {
        let diags = check_source(
            r#"
module main

fn bump(x: i32): i32 {
    x = x + 1
    return x
}
"#,
        );
        assert!(first_message(&diags).contains("cannot assign to immutable variable \"x\""));
    }

    #[test]
    fn rejects_let_initializer_type_mismatch() {
        let diags = check_source(
            r#"
module main

fn main(): i32 {
    let x: i32 = true
    return x
}
"#,
        );
        assert!(
            first_message(&diags).contains("initializer for variable \"x\" expects i32, got bool")
        );
    }

    #[test]
    fn rejects_if_condition_that_is_not_bool() {
        let diags = check_source(
            r#"
module main

fn main(): i32 {
    let x: i32 = 1
    if x {
        return 1
    }
    return 0
}
"#,
        );
        assert!(first_message(&diags).contains("if condition must be bool, got i32"));
    }

    #[test]
    fn rejects_return_type_mismatch() {
        let diags = check_source(
            r#"
module main

fn main(): i32 {
    return true
}
"#,
        );
        assert!(first_message(&diags).contains("return value expects i32, got bool"));
    }

    #[test]
    fn rejects_unknown_variable_use() {
        let diags = check_source(
            r#"
module main

fn main(): i32 {
    return missing
}
"#,
        );
        assert!(first_message(&diags).contains("unknown variable \"missing\""));
    }

    #[test]
    fn allows_builtin_option_none_variant_for_now() {
        let diags = check_source(
            r#"
module main

fn main(): i32 {
    let missing: Option<i32> = None
    return 0
}
"#,
        );
        assert!(
            diags.is_empty(),
            "expected no diagnostics, got: {:?}",
            diags.items
        );
    }

    #[test]
    fn rejects_assignment_value_type_mismatch() {
        let diags = check_source(
            r#"
module main

fn main(): i32 {
    let mut x: i32 = 1
    x = true
    return x
}
"#,
        );
        assert!(first_message(&diags).contains("assignment value expects i32, got bool"));
    }

    #[test]
    fn rejects_array_literal_length_mismatch() {
        let diags = check_source(
            r#"
module main

fn main(): i32 {
    let values: Array<i32, 2> = [1, 2, 3]
    return 0
}
"#,
        );
        assert!(first_message(&diags).contains(
            "initializer for variable \"values\" expects Array<i32, 2>, got array literal with 3 elements"
        ));
    }

    #[test]
    fn allows_function_call_with_checked_argument_and_return_types() {
        let diags = check_source(
            r#"
module main

fn id(x: i32): i32 {
    return x
}

fn main(): i32 {
    let y: i32 = id(1)
    return y
}
"#,
        );
        assert!(
            diags.is_empty(),
            "expected no diagnostics, got: {:?}",
            diags.items
        );
    }

    #[test]
    fn rejects_function_call_argument_type_mismatch() {
        let diags = check_source(
            r#"
module main

fn id(x: i32): i32 {
    return x
}

fn main(): i32 {
    return id(true)
}
"#,
        );
        assert!(
            first_message(&diags).contains("argument 1 for function \"id\" expects i32, got bool")
        );
    }

    #[test]
    fn surfaces_multiple_errors_in_one_pass() {
        // Two independent type errors; multi-error recovery should report both
        // rather than bailing at the first.
        let diags = check_source(
            r#"
module main

fn main(): i32 {
    let a: i32 = true
    let b: i32 = "hello"
    return 0
}
"#,
        );
        assert_eq!(
            diags.items.len(),
            2,
            "expected 2 diagnostics, got {}: {:?}",
            diags.items.len(),
            diags.items
        );
    }

    // ---- Phase 2/3 feature coverage ----

    fn assert_clean(diags: &Diagnostics) {
        assert!(
            diags.is_empty(),
            "expected no diagnostics, got: {:?}",
            diags.items
        );
    }

    #[test]
    fn struct_literal_and_field_access_typecheck() {
        let diags = check_source(
            r#"
module main

struct Point { x: i32 y: i32 }

fn main(): i32 {
    let p: Point = Point { x: 1, y: 2 }
    let sum: i32 = p.x + p.y
    return sum
}
"#,
        );
        assert_clean(&diags);
    }

    #[test]
    fn rejects_struct_literal_wrong_field_type() {
        let diags = check_source(
            r#"
module main

struct Point { x: i32 y: i32 }

fn main(): i32 {
    let p: Point = Point { x: true, y: 2 }
    return 0
}
"#,
        );
        assert!(first_message(&diags).contains("struct field \"x\" expects i32, got bool"));
    }

    #[test]
    fn rejects_struct_literal_unknown_field() {
        let diags = check_source(
            r#"
module main

struct Point { x: i32 y: i32 }

fn main(): i32 {
    let p: Point = Point { x: 1, y: 2, z: 3 }
    return 0
}
"#,
        );
        assert!(first_message(&diags).contains("struct \"Point\" has no field \"z\""));
    }

    #[test]
    fn rejects_assignment_to_immutable_struct_field() {
        let diags = check_source(
            r#"
module main

struct Point { x: i32 }

fn main(): i32 {
    let p: Point = Point { x: 1 }
    p.x = 5
    return 0
}
"#,
        );
        assert!(first_message(&diags).contains("cannot assign to immutable variable \"p\""));
    }

    #[test]
    fn for_in_over_array_typechecks() {
        let diags = check_source(
            r#"
module main

fn main(): i32 {
    let nums: Array<i32, 3> = [1, 2, 3]
    let mut sum: i32 = 0
    for n in nums {
        sum += n
    }
    return sum
}
"#,
        );
        assert_clean(&diags);
    }

    #[test]
    fn for_in_over_vec_typechecks() {
        let diags = check_source(
            r#"
module main

fn main(): i32 {
    let v: Vec<i32> = vec_new()
    let mut sum: i32 = 0
    for n in v {
        sum += n
    }
    return sum
}
"#,
        );
        assert_clean(&diags);
    }

    #[test]
    fn rejects_for_in_over_non_collection() {
        let diags = check_source(
            r#"
module main

fn main(): i32 {
    let x: i32 = 5
    for n in x {
        return n
    }
    return 0
}
"#,
        );
        assert!(first_message(&diags).contains("for-in expects Slice<T> or Array<T, N>, got i32"));
    }

    #[test]
    fn rejects_indexing_a_non_array() {
        let diags = check_source(
            r#"
module main

fn main(): i32 {
    let x: i32 = 5
    let y: i32 = x[0]
    return y
}
"#,
        );
        assert!(first_message(&diags).contains("cannot index into i32"));
    }

    #[test]
    fn rejects_non_integer_array_index() {
        let diags = check_source(
            r#"
module main

fn main(): i32 {
    let nums: Array<i32, 3> = [1, 2, 3]
    let y: i32 = nums[true]
    return y
}
"#,
        );
        assert!(first_message(&diags).contains("array index must be an integer, got bool"));
    }

    #[test]
    fn rejects_compound_assignment_with_wrong_type() {
        // `sum += true` desugars to `sum = sum + true`; the + on a bool errors.
        let diags = check_source(
            r#"
module main

fn main(): i32 {
    let mut sum: i32 = 0
    sum += true
    return sum
}
"#,
        );
        assert!(first_message(&diags).contains("right operand of + must be numeric, got bool"));
    }

    #[test]
    fn match_on_option_typechecks() {
        let diags = check_source(
            r#"
module main

fn f(o: Option<i32>): i32 {
    match o {
        Some(v) => { return v }
        None => { return 0 }
    }
}

fn main(): i32 {
    return 0
}
"#,
        );
        assert_clean(&diags);
    }
}
