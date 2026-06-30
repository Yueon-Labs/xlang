module main

// fold [-w N] [file] — wrap lines at N columns (default 80). GNU -w flag.
fn main(): i32 {
    let mut width: i32 = 80
    let mut file: String = ""
    let mut i: i32 = 1
    while i < argc() {
        let a: String = argv(i)
        let c0: i32 = str_char_at(a, 0)
        if c0 == 45 {
            let c1: i32 = str_char_at(a, 1)
            if c1 == 119 {
                if str_len(a) > 2 {
                    width = str_to_int(str_slice(a, 2, str_len(a)))
                } else {
                    i = i + 1
                    if i < argc() {
                        width = str_to_int(argv(i))
                    }
                }
            }
        } else {
            file = a
        }
        i = i + 1
    }
    let mut s: String = ""
    if str_len(file) > 0 {
        s = read_file(file)
    } else {
        s = read_stdin()
    }
    let n: i32 = str_len(s)
    let mut col: i32 = 0
    let mut k: i32 = 0
    while k < n {
        let c: i32 = str_char_at(s, k)
        print_raw(str_slice(s, k, k + 1))
        if c == 10 {
            col = 0
        } else {
            col = col + 1
            if col >= width {
                print_raw("\n")
                col = 0
            }
        }
        k = k + 1
    }
    return 0
}
