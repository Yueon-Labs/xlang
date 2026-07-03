module main

fn is_digit(c: i32): bool {
    return c >= '0' && c <= '9'
}

fn main(): i32 {
    print_i32('A')
    print_i32('a')
    print_i32('\n')
    print_i32('\t')
    print_i32('\\')
    print_i32('\'')
    if is_digit(str_char_at("x5y", 1)) { print_str("digit\n") }
    if str_char_at("hi", 0) == 'h' { print_str("match\n") }
    return 0
}
