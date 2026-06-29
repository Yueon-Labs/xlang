# Error Code Catalogue

`xlangc` diagnostics carry a stable `ErrorCode` that serializes to a fixed
`EXXXX` string (see `src/error.rs`). **This file is the frozen contract.**
Once a code ships, its serialized string never changes — AI tooling and autofix
engines dispatch on it.

| Code | Variant | Title | Autofix strategy |
|------|---------|-------|------------------|
| `E1001` | `LexUnexpectedChar` | Unexpected character | Drop or replace the offending character; rarely auto-fixable. |
| `E1002` | `LexUnterminatedString` | Unterminated string literal | Insert a closing `"`. |
| `E2001` | `ParseUnexpectedToken` | Unexpected token (generic) | Context-specific; usually surface for manual edit. |
| `E2002` | `ParseExpectedToken` | Expected a specific token | Insert the expected token (e.g. missing `)` `,` `:` `>`). |
| `E2003` | `ParseExpectedIdent` | Expected an identifier | Replace the token with a valid identifier. |
| `E2004` | `ParseUnterminatedBlock` | Unterminated block | Insert the closing `}`. |
| `E2005` | `ParseExpectedExpression` | Expected an expression | Replace with a valid expression. |
| `E2006` | `ParseUnknownItem` | Expected a top-level item | Start the item with `struct` / `type` / `fn`. |
| `E3001` | `TypeUnknownVar` | Unknown variable | Declare it, or fix a typo (suggest nearest in-scope name). |
| `E3002` | `TypeImmutableAssign` | Assignment to immutable variable | Add `mut`: `let x` → `let mut x`. |
| `E3003` | `TypeUnknownAssignTarget` | Assignment to unknown variable | Declare it, or fix a typo. |
| `E3004` | `TypeAssignmentTarget` | Invalid assignment target | Make the left side a variable or field access. |
| `E3005` | `TypeMismatch` | Type mismatch | Adjust the declared type or the value to make them compatible. |
| `E3006` | `TypeArgCount` | Wrong number of call arguments | Add/remove arguments to match the signature. |
| `E3007` | `TypeBoolRequired` | Condition must be `bool` | Wrap in a comparison (`x == 0`) or supply a `bool`. |
| `E3008` | `TypeNumericRequired` | Operand must be numeric | Supply a numeric value/expression. |
| `E3009` | `TypeOperatorMismatch` | Operator cannot combine types | Make both operands the same numeric type. |
| `E3010` | `TypeForInExpectsSlice` | `for` expects `Slice<T>` | Iterate over a `Slice<T>`. |
| `E3011` | `TypeReturnMissingValue` | `return` missing a value | Add a value matching the return type. |
| `E9001` | `CodegenUnsupported` | Feature not supported by the C backend | Restrict to the supported subset (no autofix). |
| `E9002` | `Internal` | Internal compiler error | File a bug (no autofix). |

## Numbering

- `E1xxx` — lexer
- `E2xxx` — parser
- `E3xxx` — type checker
- `E9xxx` — codegen / internal

## Stability rules

1. The serialized string (e.g. `"E3005"`) is immutable once released.
2. Adding a new code appends a new row; never renumber an existing one.
3. The `ErrorCode` enum is exhaustive — adding a variant forces every `match`
   on it to handle the new code, so autofix tables can't silently go stale.
4. Autofix `TextEdit` suggestions (LSP shape) attach to a `Diagnostic` in a
   later phase; the code is the dispatch key.
