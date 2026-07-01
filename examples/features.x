module main

// features.x — exercise xlang's modern language features in one program.
// Run: xlangc run examples/features.x

struct Point {
    x: i32
    y: i32
}

// Recursive fibonacci (i32).
fn fib(n: i32): i32 {
    if n < 2 { return n }
    return fib(n - 1) + fib(n - 2)
}

fn main(): i32 {
    // -- for-in over Vec + struct literals --
    let pts: Vec<Point> = vec_new()
    pts.push(Point { x: 3, y: 4 })
    pts.push(Point { x: 5, y: 12 })
    pts.push(Point { x: 8, y: 6 })
    let mut total_x: i32 = 0
    for p in pts {
        total_x = total_x + p.x
    }
    print_raw("total_x = ")
    print_raw(int_to_str(total_x))
    print_raw("\n")

    // -- match with integer literals + wildcard --
    let code: i32 = 2
    match code {
        0 => { print_raw("zero\n") }
        1 => { print_raw("one\n") }
        2 => { print_raw("two\n") }
        _ => { print_raw("other\n") }
    }

    // -- match with string literals --
    let word: String = "hello"
    match word {
        "hi" => { print_raw("greeting-hi\n") }
        "hello" => { print_raw("greeting-hello\n") }
        _ => { print_raw("unknown\n") }
    }

    // -- f64 arithmetic + float_to_str --
    let pi: f64 = 3.14159
    let area: f64 = pi * 2.0 * 2.0
    print_raw("circle area (r=2) = ")
    print_raw(float_to_str(area))
    print_raw("\n")

    // -- recursion --
    let f10: i32 = fib(10)
    print_raw("fib(10) = ")
    print_raw(int_to_str(f10))
    print_raw("\n")

    // -- break + continue in while --
    let mut i: i32 = 0
    sb_new()
    while i < 10 {
        i = i + 1
        if i == 3 { continue }
        if i == 8 { break }
        if i > 1 { sb_push(" ") }
        sb_push(int_to_str(i))
    }
    print_raw("break/continue: ")
    print_raw(sb_str())
    print_raw("\n")

    // -- string builtins --
    let s: String = "Hello World"
    print_raw(str_lower(s))
    print_raw(" | ")
    print_raw(str_upper(s))
    print_raw(" | ")
    print_raw(str_repeat("=", 5))
    print_raw("\n")

    // -- math builtins --
    print_raw("abs(-7)=")
    print_raw(int_to_str(abs(-7)))
    print_raw(" max(3,9)=")
    print_raw(int_to_str(max(3, 9)))
    print_raw(" min(3,9)=")
    print_raw(int_to_str(min(3, 9)))
    print_raw("\n")

    return 0
}
