# X Language Specification v0.1 Draft

X Language is an AI-first, TypeScript-like systems language designed for static analysis and high-performance compilation.

## Core principles

1. One canonical syntax per concept.
2. No dynamic escape hatch such as `any`.
3. No `null` or `undefined`; use `Option<T>`.
4. No exceptions; use `Result<T, E>`.
5. No implicit truthiness or implicit casts.
6. No hidden function calls, hidden allocation, or hidden control flow.
7. Compiler diagnostics must be clear and machine-readable.

## Modules

```x
module main

import fs
import math.vector
```

## Functions

```x
fn add(a: i32, b: i32): i32 {
    return a + b
}
```

Rules:

- Return type is required.
- Parameter types are required.
- Function overloading is not allowed in v0.1.
- Default arguments are not allowed in v0.1.

## Variables

```x
let x: i32 = 1
let mut total: i64 = 0
```

Rules:

- Variables are immutable by default.
- Mutable variables must use `let mut`.
- Type annotations are required in v0.1.

## If / else

```x
if condition {
    // statements
} else if other_condition {
    // statements
} else {
    // statements
}
```

Rules:

- The condition must have type `bool`.
- Braces are required.
- Parentheses around conditions are not used.
- Truthy/falsy conversion is not allowed.
- `if` is a statement in v0.1, not an expression.

## Structs

```x
struct User {
    id: i64
    name: String
    age: Option<i32>
}
```

Rules:

- Struct fields are statically known.
- Dynamic fields are not allowed.
- Field order is part of the ABI layout for C codegen.

## Type aliases

```x
type UserId = i64
```

## Option

```x
let age: Option<i32> = Some(18)
let missing: Option<i32> = None
```

`Option<T>` must be handled with `match` before extracting the value.

## Result

```x
fn divide(a: f64, b: f64): Result<f64, String> {
    if b == 0.0 {
        return Err("divide by zero")
    }

    return Ok(a / b)
}
```

`Result<T, E>` must be handled with `match` before extracting the value.

## Built-in scalar types

```txt
i32 i64
f32 f64
bool
String
Str
```

## Collection types

```txt
Array<T, N>
Vec<T>
Slice<T>
```

## Forbidden in v0.1

```txt
any
unknown as escape hatch
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
