module main

// head [-n N | -N] [file] — first N lines (default 10), GNU-compatible flags.
// Handles `head -3 f`, `head -n 3 f`, `head -n3 f`, `head f`, `head N f`.
fn main(): i32 {
    let mut limit: i32 = 10
    let mut file: String = ""
    let mut i: i32 = 1
    while i < argc() {
        let a: String = argv(i)
        let c0: i32 = str_char_at(a, 0)
        if c0 == 45 {
            let c1: i32 = str_char_at(a, 1)
            if c1 == 110 {
                if str_len(a) > 2 {
                    limit = str_to_int(str_slice(a, 2, str_len(a)))
                } else {
                    i = i + 1
                    if i < argc() {
                        limit = str_to_int(argv(i))
                    }
                }
            } else {
                if c1 >= 48 {
                    if c1 <= 57 {
                        limit = str_to_int(str_slice(a, 1, str_len(a)))
                    }
                }
            }
        } else {
            if c0 >= 48 {
                if c0 <= 57 {
                    limit = str_to_int(a)
                }
            } else {
                file = a
            }
        }
        i = i + 1
    }
    let mut s: String = ""
    if str_len(file) > 0 {
        s = read_file(file)
    } else {
        s = read_stdin()
    }
    let n: i32 = str_len(s)
    let mut printed: i32 = 0
    let mut start: i32 = 0
    let mut k: i32 = 0
    while k < n {
        if str_char_at(s, k) == 10 {
            print_raw(str_slice(s, start, k))
            print_raw("\n")
            printed = printed + 1
            start = k + 1
            if printed >= limit {
                return 0
            }
        }
        k = k + 1
    }
    if start < n {
        if printed < limit {
            print_raw(str_slice(s, start, n))
            print_raw("\n")
        }
    }
    return 0
}
