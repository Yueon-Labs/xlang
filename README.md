# NTS

**NTS** is an experimental AI-first, high-performance, TypeScript-like systems language.

The goal is simple:

> Keep the readable parts of TypeScript, remove the unsafe/dynamic parts, and compile to fast native/Wasm targets.

NTS is currently in the **v0.1 language-design phase**. The first prototype target is:

```txt
.nts source -> parser -> AST -> type checker -> C codegen -> executable
```

## Design goals

- AI-friendly: one canonical way to write each construct.
- High performance: fixed layouts, static typing, C/Wasm-friendly semantics.
- No `any`.
- No `null` / `undefined`.
- No exceptions; use `Result<T, E>`.
- No JS magic behavior: no implicit truthiness, no dynamic objects, no hidden calls.
- Clear machine-readable compiler diagnostics for AI auto-repair.

## Example

```nts
module main

fn main(): i32 {
    let age: i32 = 20

    if age >= 18 {
        return 1
    } else {
        return 0
    }
}
```

## v0.1 language surface

Planned v0.1 features:

```txt
module
import
struct
type alias
fn
let / let mut
if / else
for in
while
match
return
break / continue

i32 i64 f32 f64 bool String Str
Array<T, N> Vec<T> Slice<T>
Option<T> Result<T, E>
```

Explicitly out of scope for v0.1:

```txt
any
null
undefined
exceptions
class
this
inheritance
prototype
dynamic object fields
implicit casts
truthy/falsy conditions
eval
reflection
async/await
closures
macros
```

## Repository layout

```txt
docs/       Language specification and design notes
examples/   Hand-written .nts examples used to validate syntax
```

## Current status

This repository is intentionally small. The immediate milestone is to lock the v0.1 spec and examples before implementing the parser.

## License

MIT License. See [LICENSE](./LICENSE).
