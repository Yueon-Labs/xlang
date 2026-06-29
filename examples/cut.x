module main

// cut <delim> <field> [file] — extract the Nth delimited field per line
// (like `cut -d<delim> -f<field>`). stdin if no file.
fn main(): i32 {
    if argc() < 3 {
        print_str("usage: cut <delim> <field> [file]")
        return 1
    }
    let delim: String = argv(1)
    let field: i32 = str_to_int(argv(2))
    let mut s: String = ""
    if argc() >= 4 {
        s = read_file(argv(3))
    } else {
        s = read_stdin()
    }
    let d: i32 = str_char_at(delim, 0)
    let n: i32 = str_len(s)
    let mut lstart: i32 = 0
    let mut i: i32 = 0
    while i <= n {
        if i == n || str_char_at(s, i) == 10 {
            let line: String = str_slice(s, lstart, i)
            let ln: i32 = str_len(line)
            let mut fstart: i32 = 0
            let mut cur: i32 = 1
            let mut j: i32 = 0
            while j <= ln {
                if j == ln || str_char_at(line, j) == d {
                    if cur == field {
                        print_raw(str_slice(line, fstart, j))
                        print_raw("\n")
                    }
                    cur += 1
                    fstart = j + 1
                }
                j += 1
            }
            lstart = i + 1
        }
        i += 1
    }
    return 0
}
