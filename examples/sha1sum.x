module main

// sha1sum [file] — print SHA-1 hash (like GNU sha1sum).

fn main(): i32 {
    let mut file: String = ""
    if argc() >= 2 {
        file = argv(1)
    }
    let mut s: String = ""
    if str_len(file) > 0 {
        s = read_file(file)
    } else {
        s = read_stdin()
    }
    print_raw(sha1_hex(s))
    print_raw("  ")
    if str_len(file) > 0 {
        print_raw(file)
    } else {
        print_raw("-")
    }
    print_raw("\n")
    return 0
}
