module main

// od [file] — hex byte dump (like `od -An -tx1`). Uses bitwise ops to split
// each byte into high/low nibbles, looked up in the hex alphabet. stdin if no
// file.
fn main(): i32 {
    let mut s: String = ""
    if argc() >= 2 {
        s = read_file(argv(1))
    } else {
        s = read_stdin()
    }
    let n: i32 = str_len(s)
    let hex: String = "0123456789abcdef"
    let mut i: i32 = 0
    while i < n {
        let b: i32 = str_char_at(s, i)
        let hi: i32 = b >> 4
        let lo: i32 = b & 15
        print_raw(str_slice(hex, hi, hi + 1))
        print_raw(str_slice(hex, lo, lo + 1))
        print_raw(" ")
        i += 1
        if i % 16 == 0 {
            print_raw("\n")
        }
    }
    if n > 0 {
        if n % 16 != 0 {
            print_raw("\n")
        }
    }
    return 0
}
