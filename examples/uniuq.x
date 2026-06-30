module main

// uniqu [file] — only print lines that appear exactly once (like `uniq -u`).
// Complement of uniqd. stdin if no file.
fn main(): i32 {
    let mut s: String = ""
    if argc() >= 2 {
        s = read_file(argv(1))
    } else {
        s = read_stdin()
    }
    let n: i32 = str_len(s)
    let mut prev: String = ""
    let mut count: i32 = 0
    let mut first: bool = true
    let mut start: i32 = 0
    let mut i: i32 = 0
    while i < n {
        if str_char_at(s, i) == 10 {
            let line: String = str_slice(s, start, i)
            if first {
                prev = line
                count = 1
                first = false
            } else {
                if str_eq(line, prev) {
                    count += 1
                } else {
                    if count == 1 {
                        print_raw(prev)
                        print_raw("\n")
                    }
                    prev = line
                    count = 1
                }
            }
            start = i + 1
        }
        i += 1
    }
    if start < n {
        let line: String = str_slice(s, start, n)
        if first {
            count = 1
        } else {
            if str_eq(line, prev) {
                count += 1
            } else {
                if count == 1 {
                    print_raw(prev)
                    print_raw("\n")
                }
                prev = line
                count = 1
            }
        }
    }
    if count == 1 {
        print_raw(prev)
        print_raw("\n")
    }
    return 0
}
