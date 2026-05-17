# X Language AST v0.1 Draft

The parser should produce a simple, stable AST that is easy for type checking and JSON diagnostics.

## Top-level

```ts
type Program = {
  kind: "Program"
  module: ModuleDecl
  imports: ImportDecl[]
  items: Item[]
}

type Item = StructDecl | TypeAliasDecl | FnDecl
```

## Statements

```ts
type Stmt =
  | LetStmt
  | IfStmt
  | ForStmt
  | WhileStmt
  | MatchStmt
  | ReturnStmt
  | BreakStmt
  | ContinueStmt
  | ExprStmt
```

## If statement

```ts
type IfStmt = {
  kind: "IfStmt"
  condition: Expr
  thenBlock: Block
  elseBranch: Block | IfStmt | null
}
```

Example source:

```x
if age >= 18 {
    return 1
} else {
    return 0
}
```

Example AST shape:

```json
{
  "kind": "IfStmt",
  "condition": {
    "kind": "BinaryExpr",
    "op": ">=",
    "left": { "kind": "Identifier", "name": "age" },
    "right": { "kind": "IntLiteral", "value": "18" }
  },
  "thenBlock": {
    "kind": "Block",
    "statements": [
      { "kind": "ReturnStmt", "value": { "kind": "IntLiteral", "value": "1" } }
    ]
  },
  "elseBranch": {
    "kind": "Block",
    "statements": [
      { "kind": "ReturnStmt", "value": { "kind": "IntLiteral", "value": "0" } }
    ]
  }
}
```
