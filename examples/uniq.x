module main

// uniq <file> — drop adjacent duplicate lines (like GNU uniq). First line
// always printed; subsequent lines printed only if different from the previous.
fn main(): i32 {
    let s: String = read_file(argv(1))
    let n: i32 = str_len(s)
    let mut start: i32 = 0
    let mut prev: String = ""
    let mut first: bool = true
    let mut i: i32 = 0
    while i < n {
        if str_char_at(s, i) == 10 {
            let line: String = str_slice(s, start, i)
            if first {
                print_raw(line)
                print_raw("\n")
                prev = line
                first = false
            } else {
                if !str_eq(line, prev) {
                    print_raw(line)
                    print_raw("\n")
                    prev = line
                }
            }
            start = i + 1
        }
        i += 1
    }
    return 0
}
