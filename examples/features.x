module main

// features.x — exercise xlang's modern language features in one program.
// Run: xlangc run examples/features.x

// A unit-variant enum: a set of named constants of type Color.
enum Color {
    Red
    Green
    Blue
}

// A payload enum (sum type): Error carries a String, Ok carries nothing.
enum Status {
    Ok
    Error(String)
}

// Methods on Color reuse the impl machinery (the receiver's type dispatches).
impl Color {
    fn name(self: Color): String {
        match self {
            Red => { return "red" }
            Green => { return "green" }
            Blue => { return "blue" }
        }
    }
}

struct Point {
    x: i32
    y: i32
}

// Methods on Point: `impl Type { fn name(self: Type, ...): T { ... } }`.
// Methods compile to mangled free functions; `p.method()` dispatches via the
// receiver's type. Chained calls work (a method returning Point can be followed
// by another method).
impl Point {
    fn length_sq(self: Point): i32 {
        return self.x * self.x + self.y * self.y
    }
    fn with_offset(self: Point, dx: i32, dy: i32): Point {
        return Point { x: self.x + dx, y: self.y + dy }
    }
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

    // -- methods (impl blocks) --
    let origin: Point = Point { x: 0, y: 0 }
    let corner: Point = origin.with_offset(3, 4)
    print_raw("length_sq(3,4) = ")
    print_raw(int_to_str(corner.length_sq()))       // 25
    print_raw("  chained = ")
    print_raw(int_to_str(origin.with_offset(5, 12).length_sq()))  // 169
    print_raw("\n")

    // -- match with integer literals + wildcard --
    let code: i32 = 2
    match code {
        0 => { print_raw("zero\n") }
        1 => { print_raw("one\n") }
        2 => { print_raw("two\n") }
        _ => { print_raw("other\n") }
    }

    // -- match with OR-patterns and ranges --
    let digit: i32 = 7
    match digit {
        0 | 1 => { print_raw("boolean-ish\n") }
        2..=9 => { print_raw("single digit\n") }   // 7 matches here
        10..99 => { print_raw("two digits\n") }
        _ => { print_raw("big\n") }
    }

    // -- match with string literals --
    let word: String = "hello"
    match word {
        "hi" => { print_raw("greeting-hi\n") }
        "hello" => { print_raw("greeting-hello\n") }
        _ => { print_raw("unknown\n") }
    }

    // -- unit-variant enum (+ methods, via impl) --
    let c: Color = Green
    match c {
        Red => { print_raw("color: red\n") }
        Green => { print_raw("color: green\n") }
        Blue => { print_raw("color: blue\n") }
    }
    print_str("via method: " + c.name() + "\n")   // via method: green

    // -- payload enum (sum type): a variant carrying data --
    let st: Status = Error("disk full")
    match st {
        Ok => { print_raw("status: ok\n") }
        Error(msg) => { print_raw("status: " + msg + "\n") }   // status: disk full
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

    // -- numeric range for-loop (for i in start..end, exclusive) --
    // Lowers to C `for (i = start; i < end; i++)`. Compound assignment (+=, *=)
    // desugars to `x = x <op> y`.
    let mut sum: i32 = 0
    for i in 0..101 {
        sum += i
    }
    print_raw("sum 0..100 = ")
    print_raw(int_to_str(sum))
    print_raw("\n")

    // range with an expression end + compound *=: factorial of 6 = 720
    let mut fact: i32 = 1
    let n: i32 = 7
    for k in 1..n {
        fact *= k
    }
    print_raw("6! = ")
    print_raw(int_to_str(fact))
    print_raw("\n")

    // -- string builtins --
    let s: String = "Hello World"
    print_raw(str_lower(s))
    print_raw(" | ")
    print_raw(str_upper(s))
    print_raw(" | ")
    print_raw(str_repeat("=", 5))
    print_raw("\n")

    // -- string concatenation with + (driven by operand types) --
    // `+` on strings lowers to str_concat; `+` on ints stays numeric. The
    // compiler knows the type of every expression, so `"a" + b + 1` is a
    // clean type error (String + i32), not silent pointer arithmetic.
    let who: String = "World"
    print_raw("Hello, " + who + "!" + " (" + int_to_str(str_len(who)) + " letters)")
    print_raw("\n")

    // -- string comparison (`< <= > >= == !=` → strcmp) --
    // Lexicographic ordering and content equality. `==` compares content, not
    // pointers, so two equal strings test equal.
    let x: String = "apple"
    let y: String = "banana"
    let z: String = "apple"
    if x < y { print_raw("apple<banana\n") }
    if x == z { print_raw("apple==apple (content)\n") }

    // -- math builtins --
    print_raw("abs(-7)=")
    print_raw(int_to_str(abs(-7)))
    print_raw(" max(3,9)=")
    print_raw(int_to_str(max(3, 9)))
    print_raw(" min(3,9)=")
    print_raw(int_to_str(min(3, 9)))
    print_raw("\n")

    // -- based integer literals (hex 0x, binary 0b, octal 0o) + bitwise --
    // Systems-code idiom: bitmasks. Literals parse to their decimal value and
    // lower to plain C integer literals.
    let mask: i32 = 0xFF
    let bits: i32 = 0b1100
    print_raw("0xFF=")
    print_raw(int_to_str(mask))
    print_raw(" 0b1100=")
    print_raw(int_to_str(bits))
    print_raw(" 0xF0|0x0F=")
    print_raw(int_to_str(0xF0 | 0x0F))
    print_raw(" 0xFF&0x0F=")
    print_raw(int_to_str(0xFF & 0x0F))
    print_raw("\n")

    // -- inclusive numeric range (for i in a..=b) --
    // sum 1..=5 = 15. Exclusive `..` shown earlier (sum 0..100).
    let mut inc: i32 = 0
    for k in 1..=5 {
        inc += k
    }
    print_raw("sum 1..=5 = ")
    print_raw(int_to_str(inc))
    print_raw("\n")

    return 0
}
