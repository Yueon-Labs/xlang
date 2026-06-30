module main

// csplit <file> <pattern> — split a file at the first line matching <pattern>
// (simplified GNU csplit: only one pattern, literal match).
// Writes: xx00 (before match) and xx01 (from match to EOF).

fn main(): i32 {
    if argc() < 3 {
        print_str("usage: csplit <file> <pattern>")
        print_raw("\n")
        return 1
    }
    let s: String = read_file(argv(1))
    let pat: String = argv(2)
    let n: i32 = str_len(s)
    let plen: i32 = str_len(pat)
    let mut split_pos: i32 = -1
    let mut start: i32 = 0
    let mut i: i32 = 0
    while i <= n {
        let at_nl: bool = i < n && str_char_at(s, i) == 10
        let at_end: bool = i == n && start < n
        if !at_nl && !at_end {
            i = i + 1
            continue
        }
        let line: String = str_slice(s, start, i)
        if split_pos < 0 {
            if str_len(line) >= plen {
                let mut matched: bool = true
                let mut k: i32 = 0
                while k < plen {
                    if str_char_at(line, k) != str_char_at(pat, k) {
                        matched = false
                    }
                    k = k + 1
                }
                if matched {
                    split_pos = start
                }
            }
        }
        start = i + 1
        i = i + 1
    }
    if split_pos < 0 {
        write_file("xx00", s)
        print_str("0\n")
        return 0
    }
    write_file("xx00", str_slice(s, 0, split_pos))
    write_file("xx01", str_slice(s, split_pos, n))
    print_i32(str_len(str_slice(s, 0, split_pos)))
    print_raw("\n")
    print_i32(str_len(str_slice(s, split_pos, n)))
    print_raw("\n")
    return 0
}
