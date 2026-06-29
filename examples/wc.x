module main

// wc [file] — count lines, words, characters, and longest line (like GNU wc -lwcL).
// stdin if no file.
fn main(): i32 {
    let mut s: String = ""
    if argc() >= 2 {
        s = read_file(argv(1))
    } else {
        s = read_stdin()
    }
    let n: i32 = str_len(s)
    let mut chars: i32 = 0
    let mut lines: i32 = 0
    let mut words: i32 = 0
    let mut max_line: i32 = 0
    let mut cur_line: i32 = 0
    let mut in_word: bool = false
    let mut i: i32 = 0
    while i < n {
        let c: i32 = str_char_at(s, i)
        chars += 1
        if c == 10 {
            lines += 1
            if cur_line > max_line {
                max_line = cur_line
            }
            cur_line = 0
        } else {
            cur_line += 1
        }
        if c == 32 || c == 9 || c == 10 || c == 13 {
            in_word = false
        } else {
            if !in_word {
                words += 1
                in_word = true
            }
        }
        i += 1
    }
    if cur_line > max_line {
        max_line = cur_line
    }
    print_i32(lines)
    print_raw(" lines\n")
    print_i32(words)
    print_raw(" words\n")
    print_i32(chars)
    print_raw(" chars\n")
    print_i32(max_line)
    print_raw(" longest\n")
    return 0
}
