module main

// kill <pid> [signal] — send a signal to a process (like GNU kill).
// Default signal is 15 (SIGTERM).
fn main(): i32 {
    if argc() < 2 {
        print_str("usage: kill <pid> [signal]")
        return 1
    }
    let pid: i32 = str_to_int(argv(1))
    let sig: i32 = 15
    if argc() >= 3 {
        sig = str_to_int(argv(2))
    }
    return kill(pid, sig)
}
