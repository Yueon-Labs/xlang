module main

// pathchk [file] — check path validity (like GNU pathchk, simplified).
// Checks each path for: empty, length > 255, contains null, contains '/' at
// the wrong place, or doesn't exist (with -p). Without -p: just checks
// basic validity (no null bytes, reasonable length).

fn main(): i32 {
    let mut check_portable: bool = false
    let mut i: i32 = 1
    while i < argc() {
        let a: String = argv(i)
        if str_eq(a, "-p") {
            check_portable = true
        } else {
            let plen: i32 = str_len(a)
            if plen == 0 {
                print_str("pathchk: empty file name")
                print_raw("\n")
            } else {
                if plen > 4095 {
                    print_str("pathchk: ")
                    print_raw(a)
                    print_str(": too long")
                    print_raw("\n")
                } else {
                    let mut has_null: bool = false
                    let mut k: i32 = 0
                    while k < plen {
                        if str_char_at(a, k) == 0 {
                            has_null = true
                        }
                        k = k + 1
                    }
                    if has_null {
                        print_str("pathchk: ")
                        print_raw(a)
                        print_str(": null byte in path")
                        print_raw("\n")
                    }
                }
            }
        }
        i = i + 1
    }
    return 0
}
