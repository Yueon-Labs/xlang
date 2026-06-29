module main

// printenv <var> — print an environment variable (like GNU printenv).
fn main(): i32 {
    if argc() < 2 {
        print_str("usage: printenv <var>")
        return 1
    }
    let val: String = getenv(argv(1))
    if str_len(val) == 0 {
        return 1
    }
    print_raw(val)
    print_raw("\n")
    return 0
}
