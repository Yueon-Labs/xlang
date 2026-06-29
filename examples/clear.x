module main

// clear — clear the terminal screen (like GNU clear). Emits ANSI ESC[2J ESC[H.
fn main(): i32 {
    print_raw("\e[2J\e[H")
    return 0
}
