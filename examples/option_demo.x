module main

fn unwrap_or(age: Option<i32>, fallback: i32): i32 {
    match age {
        Some(value) => {
            return value
        }
        None => {
            return fallback
        }
    }
}

fn main(): i32 {
    let a: Option<i32> = Some(42)
    let b: Option<i32> = None
    let x: i32 = unwrap_or(a, 0)
    let y: i32 = unwrap_or(b, 99)
    return x + y
}
