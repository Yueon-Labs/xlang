module main

// xsh — a minimal shell with builtins (pwd, cd). Reads commands from stdin,
// handles pwd/cd internally, delegates everything else to system().
//   printf "pwd\ncd /tmp\npwd\n" | ./xsh
fn main(): i32 {
    while true {
        let cmd: String = read_line()
        if str_len(cmd) == 0 {
            return 0
        }
        if str_eq(cmd, "pwd") {
            print_raw(getcwd())
            print_raw("\n")
        } else {
            if str_find(cmd, "cd ") == 0 {
                chdir(str_slice(cmd, 3, str_len(cmd)))
            } else {
                system(cmd)
            }
        }
    }
    return 0
}
