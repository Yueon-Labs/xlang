module main

// tr <from> <to> — translate chars in `from` to corresponding chars in `to`
// (like GNU tr with literal char sets, no ranges). stdin if no file.
fn main(): i32 {
    if argc() < 3 {
        print_str("usage: tr <from> <to> [file]")
        return 1
    }
    let from: String = argv(1)
    let to: String = argv(2)
    let mut s: String = ""
    if argc() >= 4 {
        s = read_file(argv(3))
    } else {
        s = read_stdin()
    }
    print_raw(str_translate(s, from, to))
    return 0
}
