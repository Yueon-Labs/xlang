use crate::source::Spanned;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct Program {
    pub kind: &'static str,
    pub module: ModuleDecl,
    pub imports: Vec<ImportDecl>,
    pub items: Vec<Spanned<Item>>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ModuleDecl {
    pub kind: &'static str,
    pub path: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ImportDecl {
    pub kind: &'static str,
    pub path: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind")]
pub enum Item {
    StructDecl {
        name: String,
        fields: Vec<FieldDecl>,
    },
    TypeAliasDecl {
        name: String,
        #[serde(rename = "type")]
        ty: TypeNode,
    },
    FnDecl {
        name: String,
        params: Vec<Param>,
        #[serde(rename = "returnType")]
        return_type: TypeNode,
        body: Block,
    },
    /// `impl Type { fn method(self: Type, ...): T { ... } ... }`. Each method
    /// is a FnDecl; codegen compiles it as a free function
    /// `__xlang_method_<Type>_<name>` and dispatches `obj.name(args)` to it
    /// (prepending the receiver), using the TypeMap to find the receiver type.
    ImplDecl {
        #[serde(rename = "typeName")]
        type_name: String,
        methods: Vec<Spanned<Item>>,
    },
    /// `enum Name { A, B, C }` — a unit-variant enum (a set of named constants
    /// of type `Name`). Variants are 0-indexed by declaration order. `Name`
    /// lowers to int32_t; a variant `A` lowers to its index; `match` on a
    /// `Name` compares the index.
    EnumDecl { name: String, variants: Vec<String> },
}

#[derive(Clone, Debug, Serialize)]
pub struct FieldDecl {
    pub kind: &'static str,
    pub name: String,
    #[serde(rename = "type")]
    pub ty: TypeNode,
}

#[derive(Clone, Debug, Serialize)]
pub struct Param {
    pub kind: &'static str,
    pub name: String,
    #[serde(rename = "type")]
    pub ty: TypeNode,
}

#[derive(Clone, Debug, Serialize)]
pub struct Block {
    pub kind: &'static str,
    pub statements: Vec<Spanned<Stmt>>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind")]
pub enum Stmt {
    LetStmt {
        mutable: bool,
        name: String,
        #[serde(rename = "type")]
        ty: TypeNode,
        value: Spanned<Expr>,
    },
    IfStmt {
        condition: Spanned<Expr>,
        #[serde(rename = "thenBlock")]
        then_block: Block,
        #[serde(rename = "elseBranch")]
        else_branch: Option<ElseBranch>,
    },
    ForStmt {
        iterator: String,
        iterable: Spanned<Expr>,
        body: Block,
    },
    WhileStmt {
        condition: Spanned<Expr>,
        body: Block,
    },
    MatchStmt {
        value: Spanned<Expr>,
        arms: Vec<MatchArm>,
    },
    ReturnStmt {
        value: Option<Spanned<Expr>>,
    },
    BreakStmt,
    ContinueStmt,
    ExprStmt {
        expr: Spanned<Expr>,
    },
}

#[derive(Clone, Debug, Serialize)]
#[serde(untagged)]
pub enum ElseBranch {
    Block(Block),
    IfStmt(Box<Spanned<Stmt>>),
}

#[derive(Clone, Debug, Serialize)]
pub struct MatchArm {
    pub kind: &'static str,
    pub pattern: Pattern,
    pub body: Block,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind")]
pub enum Pattern {
    VariantPattern {
        name: String,
        bindings: Vec<String>,
    },
    LiteralPattern {
        value: String,
    },
    WildcardPattern,
    /// `a | b | c` — matches if any alternative matches (literals/ranges).
    OrPattern {
        alternatives: Vec<Pattern>,
    },
    /// `start..end` (exclusive) or `start..=end` (inclusive), integer only.
    RangePattern {
        start: String,
        end: String,
        inclusive: bool,
    },
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind")]
pub enum TypeNode {
    TypeExpr { name: String, args: Vec<TypeNode> },
    ConstTypeArg { value: String },
}

#[derive(Clone, Debug, Serialize)]
pub struct StructLiteralField {
    pub kind: &'static str,
    pub name: String,
    pub value: Spanned<Expr>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind")]
pub enum Expr {
    IntLiteral {
        value: String,
    },
    FloatLiteral {
        value: String,
    },
    StringLiteral {
        value: String,
    },
    BoolLiteral {
        value: bool,
    },
    Identifier {
        name: String,
    },
    ArrayLiteral {
        elements: Vec<Spanned<Expr>>,
    },
    BinaryExpr {
        op: String,
        left: Box<Spanned<Expr>>,
        right: Box<Spanned<Expr>>,
    },
    UnaryExpr {
        op: String,
        value: Box<Spanned<Expr>>,
    },
    AssignmentExpr {
        target: Box<Spanned<Expr>>,
        value: Box<Spanned<Expr>>,
    },
    CallExpr {
        callee: Box<Spanned<Expr>>,
        args: Vec<Spanned<Expr>>,
    },
    FieldAccessExpr {
        object: Box<Spanned<Expr>>,
        field: String,
    },
    StructLiteral {
        name: String,
        fields: Vec<StructLiteralField>,
    },
    IndexExpr {
        object: Box<Spanned<Expr>>,
        index: Box<Spanned<Expr>>,
    },
    /// Numeric range `start..end` (exclusive) or `start..=end` (inclusive).
    /// Currently only meaningful as the iterable of a `for` loop:
    /// `for i in 0..n`  → `for (i = 0; i < n; i++)`
    /// `for i in 0..=n` → `for (i = 0; i <= n; i++)`
    RangeExpr {
        start: Box<Spanned<Expr>>,
        end: Box<Spanned<Expr>>,
        inclusive: bool,
    },
}
