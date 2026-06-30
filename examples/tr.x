module main

// tr [-d] <set1> [set2] [file] — translate or delete characters.
// -d: delete all chars in set1 (like `tr -d SET`).
// Without -d: translate set1 → set2 (like `tr SET1 SET2`).
fn main(): i32 {
    let mut s: String = ""
    let mut delete_mode: bool = false
    let mut set1: String = ""
    let mut set2: String = ""

    if argc() < 2 {
        print_str("usage: tr [-d] <set1> [set2] [file]")
        return 1
    }

    if str_eq(argv(1), "-d") {
        if argc() < 3 {
            print_str("usage: tr -d <set> [file]")
            return 1
        }
        delete_mode = true
        set1 = argv(2)
        if argc() >= 4 {
            s = read_file(argv(3))
        } else {
            s = read_stdin()
        }
    } else {
        if argc() < 3 {
            print_str("usage: tr <set1> <set2> [file]")
            return 1
        }
        set1 = argv(1)
        set2 = argv(2)
        if argc() >= 4 {
            s = read_file(argv(3))
        } else {
            s = read_stdin()
        }
    }

    if delete_mode {
        let n: i32 = str_len(s)
        let sn: i32 = str_len(set1)
        let mut i: i32 = 0
        while i < n {
            let c: i32 = str_char_at(s, i)
            let mut in_set: bool = false
            let mut j: i32 = 0
            while j < sn {
                if str_char_at(set1, j) == c {
                    in_set = true
                }
                j += 1
            }
            if !in_set {
                print_raw(str_slice(s, i, i + 1))
            }
            i += 1
        }
    } else {
        print_raw(str_translate(s, set1, set2))
    }
    return 0
}
