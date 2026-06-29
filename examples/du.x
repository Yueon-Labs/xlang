module main

// du [dir] — total bytes of all files under dir, recursively (like du -sb).
// Uses file_size (stat) to get sizes without reading files.
fn du_dir(dir: String): i32 {
    let n: i32 = dir_count(dir)
    let mut i: i32 = 0
    let mut total: i32 = 0
    while i < n {
        let entry: String = dir_entry(dir, i)
        if !str_eq(entry, ".") {
            if !str_eq(entry, "..") {
                let path: String = str_concat(str_concat(dir, "/"), entry)
                if is_dir(path) {
                    total += du_dir(path)
                } else {
                    total += file_size(path)
                }
            }
        }
        i += 1
    }
    return total
}

fn main(): i32 {
    let mut dir: String = "."
    if argc() >= 2 {
        dir = argv(1)
    }
    let total: i32 = du_dir(dir)
    print_i32(total)
    print_raw(" bytes\n")
    return 0
}
