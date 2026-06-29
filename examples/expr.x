module main

// expr a op b — integer arithmetic (+ - * /), like a subset of GNU expr.
fn main(): i32 {
    if argc() < 4 {
        print_str("usage: expr a op b")
        return 1
    }
    let a: i32 = str_to_int(argv(1))
    let op: String = argv(2)
    let b: i32 = str_to_int(argv(3))
    let result: i32 = 0
    if str_eq(op, "+") {
        result = a + b
    }
    if str_eq(op, "-") {
        result = a - b
    }
    if str_eq(op, "*") {
        result = a * b
    }
    if str_eq(op, "/") {
        result = a / b
    }
    print_raw(int_to_str(result))
    print_raw("\n")
    return result
}
