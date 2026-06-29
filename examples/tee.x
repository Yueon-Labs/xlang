module main

// tee <file> — copy stdin to a file AND stdout (like GNU tee).
fn main(): i32 {
    if argc() < 2 {
        print_str("usage: tee <file>")
        return 1
    }
    let path: String = argv(1)
    let s: String = read_stdin()
    write_file(path, s)
    print_raw(s)
    return 0
}
