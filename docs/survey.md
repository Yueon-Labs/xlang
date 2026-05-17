# Related Work Survey

X Language is influenced by existing languages and tools, but aims for a different combination: AI-first syntax stability, TypeScript-like readability, strict safety, and high-performance C/Wasm targets.

## AssemblyScript

Useful for TS-like syntax and WebAssembly-oriented primitive types such as `i32`, `i64`, `f32`, and `f64`.

X Language should be stricter: no JS-style dynamic behavior, no `any`, no `null`, and no exceptions.

## MoonBit

Useful as an AI-native language/toolchain reference. X Language should learn from its focus on compiler tooling, diagnostics, and multi-backend compilation.

## Aria

Useful as an AI-code-generation-oriented language reference: no null, no exceptions, Option/Result, exhaustive matching, and explicit effects.

X Language should remain smaller in v0.1 and avoid too many advanced features.

## SoundScript

Useful for understanding how to restrict TypeScript into a safer subset.

X Language should go further by targeting native/Wasm performance rather than JS execution.

## Zig

Useful for the language philosophy: no hidden control flow, no hidden allocation, and errors as values.

X Language should adapt those ideas while using a more TypeScript-like surface syntax.

## Gleam / Grain / ReScript

Useful for no-null design, pattern matching, clear compiler errors, and Option/Result ergonomics.

## V

Useful for simple syntax, default immutability, C backend, and a pragmatic high-performance implementation route.

## Static TypeScript / MakeCode

Useful proof that a TypeScript-like subset can be statically compiled for constrained targets.

## ts-aot / MetaScript

Useful references for TypeScript-to-native ambitions. X Language should not try to compile full TypeScript; it should define a strict new language instead.
