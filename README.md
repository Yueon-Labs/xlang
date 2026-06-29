# xlang — AI-first systems language

A TypeScript-like systems language that compiles to C. Built in Rust.
Features structured diagnostics, bitwise operators, and a growing standard
library — **40 coreutils, a shell, and an HTTP server**, all written in xlang.

## Build

```sh
cargo build --release
./target/release/xlangc help
```

## Hello world

```x
module main

fn main(): i32 {
    print_str("hello from xlang")
    return 0
}
```

```sh
./target/release/xlangc c hello.x && cc -o hello hello.c && ./hello
```

## Language features

| Feature | Status |
|---------|--------|
| Scalar types (`i32 i64 f32 f64 bool String`) | ✅ |
| Parametric types (`Option<T> Result<T,E> Array<T,N> Vec<T> Slice<T>`) | ✅ |
| Structs (with literals + field access) | ✅ |
| `match` on Option/Result → C if/else | ✅ |
| Arithmetic, comparison, logical operators | ✅ |
| Bitwise operators (`& \| ^ ~ << >>`) | ✅ |
| Compound assignment (`+= -= *= /= %=`) | ✅ |
| Functions, recursion, array indexing | ✅ |
| `if/else`, `while`, `for-in` | ✅ |
| Structured diagnostics (machine-readable JSON) | ✅ |

## Standard library (~38 builtins)

| Category | Builtins |
|----------|----------|
| Console I/O | `print_i32` `print_f64` `print_str` `print_bool` `print_raw` |
| String | `str_len` `str_concat` `str_eq` `str_cmp` `str_find` `str_slice` `str_char_at` `str_reverse` `str_translate` `str_to_int` `str_to_int_oct` `int_to_str` |
| File I/O | `read_file` `write_file` `read_stdin` `read_line` |
| Filesystem | `remove_file` `rename_file` `make_dir` `chmod` `symlink` `file_size` `is_dir` |
| Directory | `dir_count` `dir_entry` |
| Networking | `tcp_listen` `accept` `recv_str` `send_str` `close_fd` |
| Process | `fork` `getpid` `argc` `argv` `system` `kill` `sleep_sec` |
| Time | `time_str` |

## Coreutils (40) — Linux userland replication

All written in xlang, compiled to C, verified against GNU on a Linux server.

```
cat  echo  wc  grep  head  tail  sort  uniq  rev  tac  tr  cut  expand
expr  tee  yes  seq  nl  factor  paste  cp  mv  rm  mkdir  chmod  ln
touch  ls  find  du  date  sleep  hostname  ps  uname  free  uptime
kill  base64  od
```

Plus **`xsh`** — a minimal shell (reads commands → executes via `system()` → loops).

## HTTP server (nginx replication)

xlang writes HTTP servers (keepalive, prefork, file serving, routing).
Benchmarked against **nginx 1.28 (from source)** on a 64-core server:

| Workload | nginx | xlang |
|----------|-------|-------|
| Fixed response, keepalive 16-conc (prefork) | 77k req/s | **129k req/s** |
| 64KB file serving, keepalive 16-conc | 25.6k req/s | 22.7k req/s |
| `cat\|grep\|sort\|uniq\|head` pipeline (500 lines) | 3ms | 5ms |

## Methodology

Built iteratively: **replicate → hit a limitation → modify xlang → implement → verify**.
Each coreutil that needed a new capability drove xlang's growth (argv, read_stdin,
str_char_at, str_cmp, bitwise operators, read_file /proc fix, Vec index-assign fix, etc.).

## Testing

52 unit tests covering every compiler component (lexer, parser, typecheck, codegen,
source, error, driver). CI green on all 71+ commits.

## License

MIT
