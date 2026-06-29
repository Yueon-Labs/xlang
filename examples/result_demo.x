module main

fn half(n: i32): Result<i32, String> {
    if n == 0 {
        return Err("zero")
    }
    return Ok(n / 2)
}

fn main(): i32 {
    let r: Result<i32, String> = half(10)
    match r {
        Ok(value) => {
            return value
        }
        Err(msg) => {
            return 0
        }
    }
}
