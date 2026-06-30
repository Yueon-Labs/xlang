module main

// tr [-d] <set1> [set2] [file] — translate or delete characters.
// -d: delete all chars in set1. Without -d: translate set1→set2.
// Delete mode builds a 256-entry lookup table (O(1) per char) and emits via
// sb_push_char (zero-alloc). Translate mode uses the O(n) str_translate builtin.
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
        let table: Vec<i32> = vec_new()
        let mut k: i32 = 0
        while k < 256 {
            table.push(0)
            k += 1
        }
        let sn: i32 = str_len(set1)
        let mut j: i32 = 0
        while j < sn {
            table[str_char_at(set1, j)] = 1
            j += 1
        }
        let n: i32 = str_len(s)
        let mut i: i32 = 0
        sb_new()
        while i < n {
            let c: i32 = str_char_at(s, i)
            if table[c] == 0 {
                sb_push_char(c)
            }
            i += 1
        }
        print_raw(sb_str())
    } else {
        print_raw(str_translate(s, set1, set2))
    }
    return 0
}
