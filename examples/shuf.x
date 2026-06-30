module main

// shuf [-n N] [file] — randomly permute lines (like GNU shuf, with optional
// -n N to output only N lines). Fisher-Yates shuffle. stdin if no file.
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
    random_seed()
    let mut limit: i32 = 0
    let mut s: String = ""
    let has_limit: bool = argc() >= 3 && str_eq(argv(1), "-n")
    if has_limit {
        limit = str_to_int(argv(2))
        if argc() >= 4 {
            s = read_file(argv(3))
        } else {
            s = read_stdin()
        }
    } else {
        if argc() >= 2 {
            s = read_file(argv(1))
        } else {
            s = read_stdin()
        }
    }
    let lines: Vec<String> = split_lines(s)
    let n: i32 = vec_len(lines)
    if limit == 0 {
        limit = n
    }
    let mut i: i32 = n - 1
    while i > 0 {
        let j: i32 = random_int(i + 1)
        let tmp: String = lines[i]
        lines[i] = lines[j]
        lines[j] = tmp
        i -= 1
    }
    let mut k: i32 = 0
    while k < limit {
        if k >= n {
            break
        }
        print_raw(lines[k])
        print_raw("\n")
        k += 1
    }
    return 0
}
