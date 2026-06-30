module main

// str_ops — exercise the string-manipulation builtin suite.

fn main(): i32 {
    // str_contains
    if str_contains("hello world", "world") == 1 {
        print_raw("contains: yes\n")
    } else {
        print_raw("contains: no\n")
    }
    if str_contains("hello world", "xyz") == 0 {
        print_raw("contains-miss: yes\n")
    }

    // str_starts_with
    if str_starts_with("foobar", "foo") == 1 {
        print_raw("starts: yes\n")
    }
    if str_starts_with("foobar", "bar") == 0 {
        print_raw("starts-miss: yes\n")
    }

    // str_ends_with
    if str_ends_with("foobar", "bar") == 1 {
        print_raw("ends: yes\n")
    }
    if str_ends_with("foobar", "foo") == 0 {
        print_raw("ends-miss: yes\n")
    }

    // str_replace
    print_raw("replace1: ")
    print_raw(str_replace("a-b-c", "-", "+"))
    print_raw("\n")
    print_raw("replace2: ")
    print_raw(str_replace("one two one", "one", "1"))
    print_raw("\n")
    // empty 'from' returns original
    print_raw("replace3: ")
    print_raw(str_replace("hello", "", "X"))
    print_raw("\n")
    // 'to' longer than 'from'
    print_raw("replace4: ")
    print_raw(str_replace("x.x", ".", "::"))
    print_raw("\n")

    return 0
}
