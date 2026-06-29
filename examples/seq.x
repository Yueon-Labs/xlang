module main

// seq — print numbers. Supports: seq LAST | seq FIRST LAST | seq FIRST STEP LAST.
// Like GNU seq. Handles positive and negative steps (countdown).
fn main(): i32 {
    if argc() < 2 {
        print_str("usage: seq <last> | <first> <last> | <first> <step> <last>")
        return 1
    }
    let first: i32 = 1
    let step: i32 = 1
    let last: i32 = 0
    if argc() == 2 {
        last = str_to_int(argv(1))
    } else {
        if argc() == 3 {
            first = str_to_int(argv(1))
            last = str_to_int(argv(2))
        } else {
            first = str_to_int(argv(1))
            step = str_to_int(argv(2))
            last = str_to_int(argv(3))
        }
    }
    if step == 0 {
        print_str("seq: step cannot be zero")
        return 1
    }
    let mut i: i32 = first
    if step > 0 {
        while i <= last {
            print_raw(int_to_str(i))
            print_raw("\n")
            i += step
        }
    } else {
        while i >= last {
            print_raw(int_to_str(i))
            print_raw("\n")
            i += step
        }
    }
    return 0
}
