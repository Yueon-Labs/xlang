module main

// ps — list running processes (PID + name) by reading /proc/[pid]/comm.
// Like a simplified `ps -e -o pid,comm`.
fn main(): i32 {
    let n: i32 = dir_count("/proc")
    let mut i: i32 = 0
    while i < n {
        let entry: String = dir_entry("/proc", i)
        let first: i32 = str_char_at(entry, 0)
        if first >= 48 {
            if first <= 57 {
                let comm: String = read_file(str_concat(str_concat("/proc/", entry), "/comm"))
                let cn: i32 = str_len(comm)
                let mut cend: i32 = cn
                if cn > 0 {
                    if str_char_at(comm, cn - 1) == 10 {
                        cend = cn - 1
                    }
                }
                print_raw(entry)
                print_raw(" ")
                print_raw(str_slice(comm, 0, cend))
                print_raw("\n")
            }
        }
        i += 1
    }
    return 0
}
