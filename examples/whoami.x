module main

// whoami — print current username (from $USER or $LOGNAME, like GNU whoami).
fn main(): i32 {
    let mut user: String = getenv("USER")
    if str_len(user) == 0 {
        user = getenv("LOGNAME")
    }
    print_raw(user)
    print_raw("\n")
    return 0
}
