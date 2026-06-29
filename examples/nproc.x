module main

// nproc — print the number of available processors (like GNU nproc).
// Counts "processor" lines in /proc/cpuinfo via repeated str_find.
fn main(): i32 {
    let s: String = read_file("/proc/cpuinfo")
    let pat: String = "processor"
    let mut count: i32 = 0
    let mut start: i32 = 0
    let n: i32 = str_len(s)
    let pn: i32 = str_len(pat)
    while start < n {
        let remaining: String = str_slice(s, start, n)
        let pos: i32 = str_find(remaining, pat)
        if pos < 0 {
            break
        }
        count += 1
        start = start + pos + pn
    }
    print_i32(count)
    print_raw("\n")
    return 0
}
