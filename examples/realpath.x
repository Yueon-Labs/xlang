module main

// realpath <path> — resolve to an absolute canonical path (like GNU realpath).
// Resolves symlinks and . / .. components.
fn main(): i32 {
    if argc() < 2 {
        print_str("usage: realpath <path>")
        return 1
    }
    let resolved: String = realpath(argv(1))
    if str_len(resolved) == 0 {
        return 1
    }
    print_raw(resolved)
    print_raw("\n")
    return 0
}
