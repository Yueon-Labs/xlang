module main

// wc [-l] [-w] [-c] [-L] [file] — count lines/words/bytes/longest-line.
// GNU-compatible flags. Single-flag output is "COUNT [file]" (matches GNU);
// no flag = lines words bytes. stdin if no file.
fn main(): i32 {
    let mut want_l: bool = false
    let mut want_w: bool = false
    let mut want_c: bool = false
    let mut want_L: bool = false
    let mut file: String = ""
    let mut i: i32 = 1
    while i < argc() {
        let a: String = argv(i)
        if str_char_at(a, 0) == 45 {
            let la: i32 = str_len(a)
            let mut j: i32 = 1
            while j < la {
                let c: i32 = str_char_at(a, j)
                if c == 108 { want_l = true }
                if c == 119 { want_w = true }
                if c == 99 { want_c = true }
                if c == 76 { want_L = true }
                j = j + 1
            }
        } else {
            file = a
        }
        i = i + 1
    }
    if !want_l {
        if !want_w {
            if !want_c {
                if !want_L {
                    want_l = true
                    want_w = true
                    want_c = true
                }
            }
        }
    }
    let mut s: String = ""
    if str_len(file) > 0 {
        s = read_file(file)
    } else {
        s = read_stdin()
    }
    let n: i32 = str_len(s)
    let mut bytes: i32 = 0
    let mut lines: i32 = 0
    let mut words: i32 = 0
    let mut max_line: i32 = 0
    let mut cur_line: i32 = 0
    let mut in_word: bool = false
    let mut k: i32 = 0
    while k < n {
        let c: i32 = str_char_at(s, k)
        bytes = bytes + 1
        if c == 10 {
            lines = lines + 1
            if cur_line > max_line {
                max_line = cur_line
            }
            cur_line = 0
        } else {
            cur_line = cur_line + 1
        }
        if c == 32 || c == 9 || c == 10 || c == 13 {
            in_word = false
        } else {
            if !in_word {
                words = words + 1
                in_word = true
            }
        }
        k = k + 1
    }
    if cur_line > max_line {
        max_line = cur_line
    }
    let mut first: bool = true
    if want_l {
        if !first { print_raw(" ") }
        print_raw(int_to_str(lines))
        first = false
    }
    if want_w {
        if !first { print_raw(" ") }
        print_raw(int_to_str(words))
        first = false
    }
    if want_c {
        if !first { print_raw(" ") }
        print_raw(int_to_str(bytes))
        first = false
    }
    if want_L {
        if !first { print_raw(" ") }
        print_raw(int_to_str(max_line))
        first = false
    }
    if str_len(file) > 0 {
        print_raw(" ")
        print_raw(file)
    }
    print_raw("\n")
    return 0
}
