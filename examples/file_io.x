module main

fn main(): i32 {
    write_file("_io_test.txt", "hello from xlang!")
    let content: String = read_file("_io_test.txt")
    print_str(content)
    return 0
}
