module main

// fold [width] [file] — wrap lines at N columns (like `fold -w N`). Default 80.
// stdin if no file.
fn main(): i32 {
    let mut width: i32 = 80
    let mut s: String = ""
    if argc() >= 2 {
        width = str_to_int(argv(1))
    }
    if argc() >= 3 {
        s = read_file(argv(2))
    } else {
        s = read_stdin()
    }
    let n: i32 = str_len(s)
    let mut col: i32 = 0
    let mut i: i32 = 0
    while i < n {
        let c: i32 = str_char_at(s, i)
        print_raw(str_slice(s, i, i + 1))
        if c == 10 {
            col = 0
        } else {
            col += 1
            if col >= width {
                print_raw("\n")
                col = 0
            }
        }
        i += 1
    }
    return 0
}
