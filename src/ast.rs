use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct Program {
    pub kind: &'static str,
    pub module: ModuleDecl,
    pub imports: Vec<ImportDecl>,
    pub items: Vec<Item>,
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
    pub statements: Vec<Stmt>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind")]
pub enum Stmt {
    LetStmt {
        mutable: bool,
        name: String,
        #[serde(rename = "type")]
        ty: TypeNode,
        value: Expr,
    },
    IfStmt {
        condition: Expr,
        #[serde(rename = "thenBlock")]
        then_block: Block,
        #[serde(rename = "elseBranch")]
        else_branch: Option<ElseBranch>,
    },
    ForStmt {
        iterator: String,
        iterable: Expr,
        body: Block,
    },
    WhileStmt {
        condition: Expr,
        body: Block,
    },
    MatchStmt {
        value: Expr,
        arms: Vec<MatchArm>,
    },
    ReturnStmt {
        value: Option<Expr>,
    },
    BreakStmt,
    ContinueStmt,
    ExprStmt {
        expr: Expr,
    },
}

#[derive(Clone, Debug, Serialize)]
#[serde(untagged)]
pub enum ElseBranch {
    Block(Block),
    IfStmt(Box<Stmt>),
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
    VariantPattern { name: String, bindings: Vec<String> },
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind")]
pub enum TypeNode {
    TypeExpr { name: String, args: Vec<TypeNode> },
    ConstTypeArg { value: String },
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
    BinaryExpr {
        op: String,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    UnaryExpr {
        op: String,
        value: Box<Expr>,
    },
    AssignmentExpr {
        target: Box<Expr>,
        value: Box<Expr>,
    },
    CallExpr {
        callee: Box<Expr>,
        args: Vec<Expr>,
    },
    FieldAccessExpr {
        object: Box<Expr>,
        field: String,
    },
}
