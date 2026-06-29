module main

// nl [file] — number each line (like GNU nl, simplified format "N line").
// stdin if no file.
fn main(): i32 {
    let mut s: String = ""
    if argc() >= 2 {
        s = read_file(argv(1))
    } else {
        s = read_stdin()
    }
    let n: i32 = str_len(s)
    let mut lineno: i32 = 1
    let mut start: i32 = 0
    let mut i: i32 = 0
    while i < n {
        if str_char_at(s, i) == 10 {
            print_raw(str_concat(str_concat(int_to_str(lineno), " "), str_slice(s, start, i)))
            print_raw("\n")
            lineno += 1
            start = i + 1
        }
        i += 1
    }
    if start < n {
        print_raw(str_concat(str_concat(int_to_str(lineno), " "), str_slice(s, start, n)))
        print_raw("\n")
    }
    return 0
}
