module main

// arch — print the machine architecture (like GNU arch, equivalent to uname -m).
fn main(): i32 {
    let m: String = uname_machine()
    if str_len(m) == 0 {
        return 1
    }
    print_raw(m)
    print_raw("\n")
    return 0
}
