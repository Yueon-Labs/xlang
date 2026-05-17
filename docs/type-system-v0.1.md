# NTS Type System v0.1 Draft

## Built-in types

```txt
i32 i64 f32 f64 bool String Str
```

## Parametric types

```txt
Option<T>
Result<T, E>
Array<T, N>
Vec<T>
Slice<T>
```

## No implicit casts

Invalid:

```nts
let x: i32 = 1.0
if 1 {
    return 1
}
```

Valid:

```nts
let x: i32 = 1
if x != 0 {
    return 1
}
```

## If condition rule

The condition of `if` and `while` must be `bool`.

Machine-readable diagnostic example:

```json
{
  "severity": "error",
  "code": "E_IF_CONDITION_NOT_BOOL",
  "message": "if condition must be bool, got i32",
  "span": { "file": "examples/if_else.nts", "start": 48, "end": 51 },
  "suggestion": "Use an explicit comparison, for example: age != 0"
}
```

## Option and Result

`Option<T>` and `Result<T, E>` values cannot be unwrapped implicitly. They must be handled using `match`.
