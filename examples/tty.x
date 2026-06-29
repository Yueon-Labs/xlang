module main

// tty — print the file name of the terminal connected to stdin (like GNU tty).
// Prints "not a tty" and returns 1 if stdin is not a terminal.
fn main(): i32 {
    let name: String = tty()
    if str_len(name) == 0 {
        print_str("not a tty")
        return 1
    }
    print_raw(name)
    print_raw("\n")
    return 0
}
