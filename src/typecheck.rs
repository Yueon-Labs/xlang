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
    /// Where the variable was declared — used to build autofix suggestions
    /// (e.g. insert `mut` for an immutable-assign error).
    decl_span: Span,
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
    /// Methods declared in `impl` blocks: (type_name, method_name) → signature
    /// (params include `self` as the first element). Used to resolve
    /// `obj.method(args)` calls.
    methods: HashMap<(String, String), FnSig>,
    /// Unit-variant enums: variant name → (enum name, index). Used to resolve
    /// a bare variant identifier (`North`) to its enum type, and (in codegen)
    /// to its integer value.
    enum_variants: HashMap<String, (String, i32, Option<CheckedType>)>,
    structs: HashMap<String, Vec<(String, CheckedType)>>,
    return_types: Vec<CheckedType>,
    diags: Diagnostics,
    /// Inferred type of every expression, keyed by the address of its
    /// `Spanned<Expr>` node. The AST is shared (by reference) between
    /// typecheck and codegen, so these heap-node addresses are stable across
    /// both passes — letting codegen look up an operand's type (e.g. to decide
    /// whether `+` is numeric add or string concat) without re-deriving it.
    types: HashMap<usize, CheckedType>,
}

/// Map of expression-node address → inferred type, produced by typecheck and
/// consumed by codegen. Keyed by `&Spanned<Expr> as *const _ as usize`.
#[derive(Default)]
pub struct TypeMap(HashMap<usize, CheckedType>);

impl TypeMap {
    /// Whether the expression at this node inferred to a string type (a `String`
    /// value or a string literal). Used by codegen to lower `+` as concatenation.
    pub fn is_string(&self, expr: &Spanned<Expr>) -> bool {
        self.0
            .get(&(expr as *const Spanned<Expr> as usize))
            .map(|t| t.is_string())
            .unwrap_or(false)
    }

    /// The named type (e.g. a user struct) of an expression, if it has one with
    /// no type arguments. Used by codegen to dispatch method calls
    /// (`obj.method()` → look up the method on obj's type).
    pub fn type_name(&self, expr: &Spanned<Expr>) -> Option<String> {
        self.0
            .get(&(expr as *const Spanned<Expr> as usize))
            .and_then(|t| match t {
                CheckedType::Named { name, args } if args.is_empty() => Some(name.clone()),
                _ => None,
            })
    }

    /// Reconstruct a `TypeNode` for an expression's inferred type, when possible
    /// (Unknown / Const / array-literal types have no node). Lets codegen bind
    /// an arbitrary expression to a typed temp — e.g. so `match func() { ... }`
    /// and `if let Pat = func() { ... }` work, not just `match var { ... }`.
    pub fn type_node(&self, expr: &Spanned<Expr>) -> Option<TypeNode> {
        self.0
            .get(&(expr as *const Spanned<Expr> as usize))
            .and_then(checked_type_to_node)
    }
}

/// Inverse of `type_from_node`: best-effort `CheckedType` → `TypeNode` for the
/// cases codegen needs to bind a typed temporary.
fn checked_type_to_node(ty: &CheckedType) -> Option<TypeNode> {
    let node = match ty {
        CheckedType::IntLiteral => TypeNode::TypeExpr {
            name: "i32".to_string(),
            args: vec![],
        },
        CheckedType::FloatLiteral => TypeNode::TypeExpr {
            name: "f64".to_string(),
            args: vec![],
        },
        CheckedType::StringLiteral => TypeNode::TypeExpr {
            name: "String".to_string(),
            args: vec![],
        },
        CheckedType::Named { name, args } => TypeNode::TypeExpr {
            name: name.clone(),
            args: args
                .iter()
                .map(checked_type_to_node)
                .collect::<Option<_>>()?,
        },
        CheckedType::Unknown | CheckedType::Const(_) | CheckedType::ArrayLiteral(_) => {
            return None;
        }
    };
    Some(node)
}

/// Type-check `program`, returning all accumulated diagnostics (empty = clean).
pub fn check_program(program: &Program) -> Diagnostics {
    check_program_typed(program).0
}

/// Like [`check_program`] but also returns the inferred type of every
/// expression ([`TypeMap`]), for codegen passes that need operand types.
pub fn check_program_typed(program: &Program) -> (Diagnostics, TypeMap) {
    let mut checker = Checker::default();
    checker.collect_functions(program);
    checker.collect_structs(program);
    checker.collect_methods(program);
    checker.collect_enums(program);
    checker.check_program(program);
    (checker.diags, TypeMap(checker.types))
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

    /// Register every `impl Type` method in the method table. params (including
    /// `self`) and return type are recorded so method calls can be resolved.
    fn collect_methods(&mut self, program: &Program) {
        for item in &program.items {
            if let Item::ImplDecl {
                type_name, methods, ..
            } = &item.node
            {
                for method in methods {
                    if let Item::FnDecl {
                        name,
                        params,
                        return_type,
                        ..
                    } = &method.node
                    {
                        self.methods.insert(
                            (type_name.clone(), name.clone()),
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
        }
    }

    /// Register every `enum Name { ... }`'s variants: variant name →
    /// (enum name, index). Variant names are global, so a later enum with a
    /// duplicate variant name shadows the earlier (matches construction
    /// resolution, which looks up the name).
    fn collect_enums(&mut self, program: &Program) {
        for item in &program.items {
            if let Item::EnumDecl { name, variants } = &item.node {
                for (idx, variant) in variants.iter().enumerate() {
                    let payload = variant.payload.as_ref().map(type_from_node);
                    self.enum_variants
                        .insert(variant.name.clone(), (name.clone(), idx as i32, payload));
                }
            }
        }
    }

    fn check_program(&mut self, program: &Program) {
        for item in &program.items {
            match &item.node {
                Item::FnDecl {
                    params,
                    return_type,
                    body,
                    ..
                } => {
                    self.check_fn_body(params, return_type, body);
                }
                Item::ImplDecl { methods, .. } => {
                    for method in methods {
                        if let Item::FnDecl {
                            params,
                            return_type,
                            body,
                            ..
                        } = &method.node
                        {
                            self.check_fn_body(params, return_type, body);
                        }
                    }
                }
                Item::StructDecl { .. } | Item::TypeAliasDecl { .. } | Item::EnumDecl { .. } => {}
            }
        }
    }

    /// Declare a function/method's params and type-check its body. Shared by
    /// top-level fns and `impl` methods (methods' first param is conventionally
    /// `self` of the impl type, but that's just an ordinary param here).
    fn check_fn_body(&mut self, params: &[Param], return_type: &TypeNode, body: &Block) {
        self.push_scope();
        self.return_types.push(type_from_node(return_type));
        for param in params {
            self.declare(
                &param.name,
                param.mutable,
                type_from_node(&param.ty),
                Span::unknown(0),
            );
        }
        self.check_statements(&body.statements);
        self.return_types.pop();
        self.pop_scope();
    }

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn declare(&mut self, name: &str, mutable: bool, ty: CheckedType, decl_span: Span) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(
                name.to_string(),
                VarInfo {
                    mutable,
                    ty,
                    decl_span,
                },
            );
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
                self.declare(name, *mutable, declared, stmt.span);
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
                // `for i in start..end`: numeric range. infer_expr validates the
                // two ends; the iterator is an i32.
                let is_range = matches!(&iterable.node, Expr::RangeExpr { .. });
                let iterable_ty = self.infer_expr(iterable);
                let iterator_ty = if is_range {
                    CheckedType::named("i32")
                } else {
                    match &iterable_ty {
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
                                    "for-in expects Slice<T>, Array<T, N>, or a..b range, got {}",
                                    other.display()
                                ),
                            );
                            CheckedType::Unknown
                        }
                    }
                };
                self.push_scope();
                self.declare(iterator, false, iterator_ty, stmt.span);
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
                    if let crate::ast::Pattern::VariantPattern { name, bindings } = &arm.pattern {
                        // Declare payload bindings with the variant's payload
                        // type (e.g. `Err(msg)` → msg: String) when known.
                        let payload_ty = self
                            .enum_variants
                            .get(name)
                            .and_then(|(_, _, p)| p.clone())
                            .unwrap_or(CheckedType::Unknown);
                        for binding in bindings {
                            self.declare(binding, false, payload_ty.clone(), stmt.span);
                        }
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
        let ty = self.infer_expr_inner(expr);
        // Record the inferred type keyed by the node's stable address, so
        // codegen can look up operand types (e.g. `+` → concat vs add).
        self.types
            .insert(expr as *const Spanned<Expr> as usize, ty.clone());
        ty
    }

    fn infer_expr_inner(&mut self, expr: &Spanned<Expr>) -> CheckedType {
        let span = expr.span;
        match &expr.node {
            Expr::IntLiteral { .. } => CheckedType::IntLiteral,
            Expr::FloatLiteral { .. } => CheckedType::FloatLiteral,
            Expr::StringLiteral { .. } => CheckedType::StringLiteral,
            Expr::BoolLiteral { .. } => CheckedType::named("bool"),
            Expr::Identifier { name } => {
                if is_builtin_variant(name) {
                    CheckedType::Unknown
                } else if let Some((enum_name, _, None)) = self.enum_variants.get(name) {
                    // A unit-variant enum constant (e.g. `North`) → its enum type.
                    CheckedType::named(enum_name)
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
                    // String/Str indexing: s[i] → i32 (byte value), like str_char_at.
                    t if t.is_string() => CheckedType::named("i32"),
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
            // A numeric range `start..end`. Only meaningful as a `for`-loop
            // iterable; infer_expr validates both ends are numeric. The range
            // itself is not a first-class value (CheckedType::Unknown), so using
            // one elsewhere is caught downstream.
            Expr::RangeExpr { start, end, .. } => {
                let start_ty = self.infer_expr(start);
                let end_ty = self.infer_expr(end);
                self.expect_numeric(&start_ty, "range start", start.span);
                self.expect_numeric(&end_ty, "range end", end.span);
                CheckedType::Unknown
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
            // String concatenation: `s1 + s2` (and `"x" + s`, etc.) desugars to
            // str_concat in codegen. If either operand is a string, the result
            // is a string; a concretely non-string operand alongside one is a
            // type error (Unknown is poison and allowed through).
            "+" if left_ty.is_string() || right_ty.is_string() => {
                if !left_ty.is_string() && !left_ty.is_unknown() {
                    self.emit(
                        span,
                        ErrorCode::TypeOperatorMismatch,
                        format!("cannot concatenate String with {}", left_ty.display()),
                    );
                }
                if !right_ty.is_string() && !right_ty.is_unknown() {
                    self.emit(
                        span,
                        ErrorCode::TypeOperatorMismatch,
                        format!("cannot concatenate String with {}", right_ty.display()),
                    );
                }
                CheckedType::named("String")
            }
            // String repeat: `s * n` (or `n * s`) → str_repeat(s, n). One
            // operand is the string, the other an integer count.
            "*" if left_ty.is_string() || right_ty.is_string() => {
                if left_ty.is_string() && right_ty.is_string() {
                    self.emit(
                        span,
                        ErrorCode::TypeOperatorMismatch,
                        "cannot repeat two strings with *".to_string(),
                    );
                }
                if left_ty.is_string() && !right_ty.is_unknown() && !right_ty.is_numeric() {
                    self.emit(
                        span,
                        ErrorCode::TypeOperatorMismatch,
                        format!("cannot repeat String with {}", right_ty.display()),
                    );
                }
                if right_ty.is_string() && !left_ty.is_unknown() && !left_ty.is_numeric() {
                    self.emit(
                        span,
                        ErrorCode::TypeOperatorMismatch,
                        format!("cannot repeat String with {}", left_ty.display()),
                    );
                }
                CheckedType::named("String")
            }
            "+" | "-" | "*" | "/" | "%" | "&" | "|" | "^" | "<<" | ">>" => {
                self.infer_numeric_result(op, &left_ty, &right_ty, span)
            }
            ">" | ">=" | "<" | "<=" => {
                // String ordering (`s1 < s2`, etc.) compares lexicographically
                // via strcmp in codegen. A concretely non-string operand
                // alongside a string is a type error; Unknown passes through.
                if left_ty.is_string() || right_ty.is_string() {
                    if !left_ty.is_string() && !left_ty.is_unknown() {
                        self.emit(
                            span,
                            ErrorCode::TypeOperatorMismatch,
                            format!(
                                "cannot compare String with {} using {op}",
                                left_ty.display()
                            ),
                        );
                    }
                    if !right_ty.is_string() && !right_ty.is_unknown() {
                        self.emit(
                            span,
                            ErrorCode::TypeOperatorMismatch,
                            format!(
                                "cannot compare String with {} using {op}",
                                right_ty.display()
                            ),
                        );
                    }
                    CheckedType::named("bool")
                } else {
                    self.expect_numeric_pair(op, &left_ty, &right_ty, span);
                    CheckedType::named("bool")
                }
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
            "~" => {
                self.expect_numeric(&value_ty, "operand of ~", value.span);
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

            // Enum payload-variant construction: `Err(msg)` → the enum type,
            // checking the payload against the variant's declared type.
            if let Some((enum_name, _, Some(payload_ty))) = self.enum_variants.get(name).cloned() {
                if args.len() == 1 {
                    let arg_ty = self.infer_expr(&args[0]);
                    self.expect_assignable(
                        &payload_ty,
                        &arg_ty,
                        &format!("payload for variant {name:?}"),
                        args[0].span,
                    );
                } else {
                    self.emit(
                        span,
                        ErrorCode::TypeArgCount,
                        format!(
                            "variant {name:?} expects 1 payload argument, got {}",
                            args.len()
                        ),
                    );
                    for arg in args {
                        self.infer_expr(arg);
                    }
                }
                return CheckedType::named(&enum_name);
            }

            for arg in args {
                self.infer_expr(arg);
            }
            // Fallback: a compiler builtin (lowered directly in codegen). Infer
            // its declared return type so dependent checks (e.g. `+`) work.
            return builtin_return_type(name).unwrap_or(CheckedType::Unknown);
        }

        // Method call: `obj.method(args)`. Resolve via the receiver's type.
        if let Expr::FieldAccessExpr { object, field } = &callee.node {
            let obj_ty = self.infer_expr(object);
            if let CheckedType::Named { name: ty_name, .. } = &obj_ty
                && let Some(sig) = self.methods.get(&(ty_name.clone(), field.clone())).cloned()
            {
                // Method params are [self, ...rest]; the call supplies the
                // rest (self comes from the receiver).
                let expected = sig.params.len().saturating_sub(1);
                if args.len() != expected {
                    self.emit(
                        span,
                        ErrorCode::TypeArgCount,
                        format!(
                            "method {ty_name:?}.{field:?} expects {expected} arguments, got {}",
                            args.len()
                        ),
                    );
                    for arg in args {
                        self.infer_expr(arg);
                    }
                    return CheckedType::Unknown;
                }
                for (index, (arg, param_ty)) in
                    args.iter().zip(sig.params.iter().skip(1)).enumerate()
                {
                    let arg_ty = self.infer_expr(arg);
                    self.expect_assignable(
                        param_ty,
                        &arg_ty,
                        &format!("argument {} for method {ty_name:?}.{field:?}", index + 1),
                        arg.span,
                    );
                }
                return sig.return_type;
            }
            // Not a known method: infer the receiver + args, return Unknown.
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
                Some(VarInfo {
                    mutable: false,
                    decl_span,
                    ..
                }) => {
                    let mut d = Diagnostic::error(
                        ErrorCode::TypeImmutableAssign,
                        span,
                        format!(
                            "cannot assign to immutable variable {name:?}; declare it with `let mut {name}` if reassignment is intended"
                        ),
                    );
                    // Autofix: insert `mut ` right after the `let ` keyword in the
                    // variable's declaration (decl_span starts at `let`, +4 bytes).
                    let insert_at =
                        Span::new(decl_span.file_id, decl_span.start + 4, decl_span.start + 4);
                    d = d.with_suggestion(insert_at, "mut ");
                    self.diags.push(d);
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
                    CheckedType::Named { name, args } if name == "Vec" && args.len() == 1 => {
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

    fn is_string(&self) -> bool {
        matches!(self, Self::StringLiteral)
            || matches!(self, Self::Named { name, args } if name == "String" && args.is_empty())
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

/// Declared return type of each compiler builtin (the functions lowered directly
/// in codegen, not user-defined). This lets the type checker infer call results
/// — so `str_len(s) + 1` typechecks as i32, and `s + str_len(s)` is correctly
/// rejected as String+i32. Names are matched only as a fallback (after
/// user-defined functions), so a user `fn str_len` shadows the builtin. Returns
/// `None` for builtins whose result type isn't worth tracking (void I/O, etc.),
/// which typecheck then treats as the Unknown poison.
fn builtin_return_type(name: &str) -> Option<CheckedType> {
    let ty = match name {
        // String-producing.
        "str_concat" | "str_lower" | "str_upper" | "str_replace" | "str_replace_first"
        | "str_repeat" | "str_slice" | "str_trim" | "str_reverse" | "str_translate" | "chr"
        | "int_to_str" | "float_to_str" | "read_file" | "read_stdin" | "recv_str" | "recv_all"
        | "rbuf_str" | "argv" | "sb_str" | "time_format" | "time_format_utc" | "time_format_at"
        | "time_format_at_utc" => CheckedType::named("String"),
        // i32-producing.
        "str_len" | "str_char_at" | "str_find" | "str_find_from" | "str_to_int"
        | "str_to_int_oct" | "str_cmp" | "argc" | "vec_len" | "abs" | "max" | "min"
        | "rbuf_byte_at" | "fork" | "wait_pid_status" | "stat_field" | "recv_n" | "read_rbuf"
        | "open_append" | "close_fd" | "seek" | "tcp_connect" | "sendfile_range" | "now_s"
        | "time_now" => CheckedType::named("i32"),
        // f64-producing.
        "str_to_float" | "int_to_f64" => CheckedType::named("f64"),
        // Note: the boolean string-search builtins (str_contains,
        // str_starts_with, str_ends_with, str_eq) are intentionally LEFT
        // UNTRACKED (None → Unknown). Their C runtime returns int 0/1, and
        // existing code uses them two ways — `str_eq(a,b) == 1` (int compare)
        // and `if str_contains(...)` (bool context). Only Unknown satisfies
        // both (expect_bool and the `==` check both pass Unknown); typing them
        // bool or i32 would break one usage. The cost is `s + str_contains(..)`
        // isn't caught — rare and benign.
        // Void / untracked — no useful value type.
        _ => return None,
    };
    Some(ty)
}

#[cfg(test)]
mod tests {
    use super::check_program;
    use crate::error::{Diagnostics, ErrorCode};
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
    fn immutable_assign_carries_let_mut_autofix() {
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
        let d = diags
            .items
            .iter()
            .find(|d| d.code == ErrorCode::TypeImmutableAssign)
            .expect("immutable-assign diagnostic");
        assert_eq!(
            d.suggestions.len(),
            1,
            "should carry one autofix suggestion"
        );
        assert_eq!(d.suggestions[0].new_text, "mut ");
        // The insert point is a zero-width range right after `let `.
        assert_eq!(d.suggestions[0].range.start, d.suggestions[0].range.end);
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
        assert!(first_message(&diags).contains("or a..b range, got i32"));
    }

    #[test]
    fn accepts_numeric_range_for_loop() {
        // `for i in 0..n` typechecks cleanly and the iterator is usable as an i32
        // (here: as an array index and in arithmetic).
        let diags = check_source(
            r#"
module main

fn main(): i32 {
    let n: i32 = 3
    let mut s: i32 = 0
    for i in 0..n {
        s = s + i
    }
    return s
}
"#,
        );
        assert!(
            diags.items.is_empty(),
            "range for-loop should typecheck: {diags:?}"
        );
    }

    #[test]
    fn rejects_non_numeric_range_bound() {
        let diags = check_source(
            r#"
module main

fn main(): i32 {
    for i in 0.."oops" {
        return i
    }
    return 0
}
"#,
        );
        assert!(
            first_message(&diags).contains("range end"),
            "should flag non-numeric range end: {diags:?}"
        );
    }

    #[test]
    fn accepts_string_concatenation() {
        // `a + b`, `"x" + a`, and chained `a + b + c` all typecheck as String.
        let diags = check_source(
            r#"
module main

fn cat(a: String, b: String): String {
    return a + b + "!"
}

fn main(): i32 { return 0 }
"#,
        );
        assert!(
            diags.items.is_empty(),
            "string + should typecheck: {diags:?}"
        );
    }

    #[test]
    fn rejects_string_plus_integer() {
        let diags = check_source(
            r#"
module main
fn main(): i32 {
    let s: String = "x"
    let r: String = s + 5
    print_str(r)
    return 0
}
"#,
        );
        assert!(
            first_message(&diags).contains("cannot concatenate String"),
            "should reject String + int: {diags:?}"
        );
    }

    #[test]
    fn rejects_string_plus_numeric_builtin() {
        // str_len is a known i32 builtin, so this is String + i32 → rejected.
        let diags = check_source(
            r#"
module main
fn main(): i32 {
    let s: String = "hello"
    let r: String = s + str_len(s)
    print_str(r)
    return 0
}
"#,
        );
        assert!(
            first_message(&diags).contains("cannot concatenate String with i32"),
            "should reject String + str_len (i32 builtin): {diags:?}"
        );
    }

    #[test]
    fn accepts_string_ordering_and_equality() {
        // `<`, `<=`, `>`, `>=`, `==`, `!=` on strings all typecheck as bool.
        let diags = check_source(
            r#"
module main
fn cmp(a: String, b: String): bool {
    return a < b && a <= b && a > b && a >= b && a == b && a != b
}
fn main(): i32 { return 0 }
"#,
        );
        assert!(
            diags.items.is_empty(),
            "string comparisons should typecheck: {diags:?}"
        );
    }

    #[test]
    fn rejects_string_ordering_with_integer() {
        let diags = check_source(
            r#"
module main
fn main(): i32 {
    let s: String = "x"
    if s < 5 { return 1 }
    return 0
}
"#,
        );
        assert!(
            first_message(&diags).contains("cannot compare String with integer literal using <"),
            "should reject String < int: {diags:?}"
        );
    }

    #[test]
    fn accepts_unit_variant_enum() {
        // An enum variant resolves to the enum type, so `return A` typechecks
        // for a function returning E, and `let e: E = B` is assignable.
        let diags = check_source(
            r#"
module main
enum E { A, B, C }
fn pick(): E { return B }
fn main(): i32 {
    let e: E = pick()
    return 0
}
"#,
        );
        assert!(diags.items.is_empty(), "enum should typecheck: {diags:?}");
    }

    #[test]
    fn accepts_and_rejects_payload_enum() {
        // A payload variant `Err(String)` constructs with a value, and match
        // binds it; a wrong-typed payload is rejected.
        let ok = check_source(
            r#"
module main
enum S { Ok, Err(String) }
fn msg(s: S): String {
    match s {
        Ok => { return "" }
        Err(m) => { return m }
    }
}
fn main(): i32 { return 0 }
"#,
        );
        assert!(ok.items.is_empty(), "payload enum should typecheck: {ok:?}");

        let bad = check_source(
            r#"
module main
enum S { Ok, Err(String) }
fn main(): i32 {
    let s: S = Err(5)
    return 0
}
"#,
        );
        assert!(
            first_message(&bad).contains("payload for variant"),
            "wrong payload type should be rejected: {bad:?}"
        );
    }

    #[test]
    fn accepts_enum_with_struct_payload() {
        // A struct payload (`Two(Pair)`) gives a multi-field variant for free:
        // the payload is the struct type, and the match binding gets its type.
        let diags = check_source(
            r#"
module main
struct Pair { a: i32
    b: i32 }
enum E { Empty, Two(Pair) }
fn sum(e: E): i32 {
    match e {
        Empty => { return 0 }
        Two(p) => { return p.a + p.b }
    }
}
fn main(): i32 { return 0 }
"#,
        );
        assert!(
            diags.items.is_empty(),
            "struct-payload enum should typecheck: {diags:?}"
        );
    }

    #[test]
    fn accepts_and_rejects_method_calls() {
        // Method call with correct arity typechecks; wrong arity is rejected.
        let ok = check_source(
            r#"
module main
struct Box { v: i32 }
impl Box { fn add(self: Box, n: i32): i32 { return self.v + n } }
fn main(): i32 {
    let b: Box = Box { v: 5 }
    return b.add(3)
}
"#,
        );
        assert!(ok.items.is_empty(), "method call should typecheck: {ok:?}");

        let bad = check_source(
            r#"
module main
struct Box { v: i32 }
impl Box { fn add(self: Box, n: i32): i32 { return self.v + n } }
fn main(): i32 {
    let b: Box = Box { v: 5 }
    return b.add(1, 2)
}
"#,
        );
        assert!(
            first_message(&bad).contains("expects 1 arguments, got 2"),
            "wrong method arity should be rejected: {bad:?}"
        );
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
