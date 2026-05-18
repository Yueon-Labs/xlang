# X Language

**X Language** is an experimental AI-first, high-performance, TypeScript-like systems language.

The goal is simple:

> Keep the readable parts of TypeScript, remove the unsafe/dynamic parts, and compile to fast native/Wasm targets.

X Language is currently in the **v0.1 language-design phase**. The first prototype target is:

```txt
.x source -> parser -> AST -> type checker -> C codegen -> executable
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

```x
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
examples/   Hand-written .x examples used to validate syntax
src/        Rust implementation of the prototype compiler
```

Compiler source layout:

```txt
src/main.rs       Thin CLI process entrypoint
src/lib.rs        Compiler library module root
src/cli.rs        Command-line argument handling
src/driver.rs     File-level compile flow: read -> parse -> C -> build
src/lexer.rs      Source text -> tokens
src/parser.rs     Tokens -> AST
src/ast.rs        AST data structures
src/codegen/c.rs  AST -> C backend
src/error.rs      Shared error/result types
```

## Prototype quick start

The repository includes a small Rust prototype compiler so the v0.1 flow can be
tried end-to-end.

```sh
# Parse all examples
make check

# Print JSON AST for the simplest executable example
cargo run --bin xlangc -- ast examples/if_else.x

# Compile examples/if_else.x to C, build it, and run it
make run-if
```

Expected `make run-if` output:

```txt
program exited with code 1
```

The exit code is `1` because `examples/if_else.x` returns `1` when `age >= 18`.

Current prototype scope:

- Lexer and parser cover the checked examples in `examples/`.
- JSON AST output is available with `cargo run --bin xlangc -- ast <file>`.
- C/native codegen currently supports the scalar subset used by
  `examples/if_else.x`.
- `Option<T>`, `Result<T, E>`, `for`, `match`, and collection lowering are parsed
  but not lowered to C yet.

## Current status

This repository is intentionally small. The immediate milestone is to lock the
v0.1 spec and examples while growing the prototype compiler in small vertical
slices.

## License

MIT License. See [LICENSE](./LICENSE).
