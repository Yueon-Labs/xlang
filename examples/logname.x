module main

// logname — print the name of the login user (from $LOGNAME, like GNU logname).
fn main(): i32 {
    let name: String = getenv("LOGNAME")
    if str_len(name) == 0 {
        return 1
    }
    print_raw(name)
    print_raw("\n")
    return 0
}
