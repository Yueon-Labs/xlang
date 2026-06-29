module main

// head [N] [file] — first N lines (default 10, like GNU head -n N). stdin if
// no file. N is detected by first-char-is-digit.
fn main(): i32 {
    let mut limit: i32 = 10
    let mut file_idx: i32 = 0
    if argc() >= 2 {
        let first: i32 = str_char_at(argv(1), 0)
        if first >= 48 {
            if first <= 57 {
                limit = str_to_int(argv(1))
                if argc() >= 3 {
                    file_idx = 2
                }
            } else {
                file_idx = 1
            }
        } else {
            file_idx = 1
        }
    }
    let mut s: String = ""
    if file_idx > 0 {
        s = read_file(argv(file_idx))
    } else {
        s = read_stdin()
    }
    let n: i32 = str_len(s)
    let mut printed: i32 = 0
    let mut start: i32 = 0
    let mut i: i32 = 0
    while i < n {
        if str_char_at(s, i) == 10 {
            print_raw(str_slice(s, start, i))
            print_raw("\n")
            printed += 1
            start = i + 1
            if printed >= limit {
                return 0
            }
        }
        i += 1
    }
    if start < n {
        if printed < limit {
            print_raw(str_slice(s, start, n))
            print_raw("\n")
        }
    }
    return 0
}
