module main

// sort <file> — sort lines lexicographically (like GNU sort). Bubble sort on a
// Vec<String> using str_cmp for ordering + vec_len for the count.
fn main(): i32 {
    let s: String = read_file(argv(1))
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
    let count: i32 = vec_len(lines)
    let mut a: i32 = 0
    while a < count {
        let mut b: i32 = 0
        while b < count - 1 - a {
            if str_cmp(lines[b], lines[b + 1]) > 0 {
                let tmp: String = lines[b]
                lines[b] = lines[b + 1]
                lines[b + 1] = tmp
            }
            b += 1
        }
        a += 1
    }
    let mut k: i32 = 0
    while k < count {
        print_raw(lines[k])
        print_raw("\n")
        k += 1
    }
    return 0
}
