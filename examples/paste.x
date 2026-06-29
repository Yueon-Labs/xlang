module main

// paste <file1> <file2> — merge lines from two files, tab-separated (like GNU
// paste). Uses a helper function with Vec<String> for line splitting.
fn split_lines(s: String): Vec<String> {
    let lines: Vec<String> = vec_new()
    let n: i32 = str_len(s)
    let mut start: i32 = 0
    let mut i: i32 = 0
    while i < n {
        if str_char_at(s, i) == 10 {
            lines.push(str_slice(s, start, i))
            start = i + 1
        }
        i += 1
    }
    if start < n {
        lines.push(str_slice(s, start, n))
    }
    return lines
}

fn main(): i32 {
    if argc() < 3 {
        print_str("usage: paste <file1> <file2>")
        return 1
    }
    let a: Vec<String> = split_lines(read_file(argv(1)))
    let b: Vec<String> = split_lines(read_file(argv(2)))
    let an: i32 = vec_len(a)
    let bn: i32 = vec_len(b)
    let mut i: i32 = 0
    while i < an || i < bn {
        if i < an {
            print_raw(a[i])
        }
        print_raw("\t")
        if i < bn {
            print_raw(b[i])
        }
        print_raw("\n")
        i += 1
    }
    return 0
}
