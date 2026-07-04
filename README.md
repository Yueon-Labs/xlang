# xlang — AI-first systems language

A TypeScript-like systems language that compiles to C via `xlangc`. Built in Rust.

**126 coreutils**, **14 HTTP/HTTPS servers**, **130 examples** — all written in xlang, compiled to C, verified against GNU on Linux CI.

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
./target/release/xlangc run hello.x
```

## Language features

| Feature | Example |
|---------|---------|
| Scalar types (`i32 i64 f64 bool String`) | `let x: i32 = 42` |
| Parametric types | `Option<T> Result<T,E> Array<T,N> Vec<T> Slice<T>` |
| Structs + methods (incl. `mut self`) | `impl Stack { fn push(mut self: Stack, v)` |
| Enums (unit + payload + recursive) | `enum Tree { Leaf, Branch(BranchData) }` |
| `match` (literals, variants, ranges, OR, negative) | `match n { -1 => ..., 2..=9 => ..., _ => ... }` |
| String operators (`+ * < <= > >= == !=`) | `"x" + "lang"`, `"=" * 10`, `s1 < s2` |
| String indexing | `s[i]` → byte value (i32) |
| Char literals | `'A'`, `'\n'`, `'\\'` |
| Based integer literals | `0xFF`, `0b1010`, `0o755` |
| Range loops | `for i in 0..n`, `for i in 0..=5` |
| `if let` / `while let` | `if let Some(v) = opt { ... }` |
| Compound assignment | `+= -= *= /= %=` |
| Bitwise operators | `& | ^ ~ << >>` |
| `assert` / `panic` / `unreachable` | `assert(x == 5)` |
| Structured diagnostics | `check --format json` (machine-readable) |
| `check --fix` autofix | machine-applicable suggestions (e.g. immutable-assign → add `mut`) |
| LSP + VSCode extension | hover, go-to-definition, completion, **documentSymbol**, **references**, **foldingRange**, live diagnostics |

### Vec API (complete)

```x
let v: Vec<i32> = vec_new()
v.push(10)
v.push(30)
v.insert(1, 20)          // [10, 20, 30]
let top: i32 = v.pop()   // 30
let len: i32 = v.len()
let x: i32 = v[0]        // 10
v.remove_at(0)            // [20]
```

### Mutable data structures

```x
struct Stack { items: Vec<i32> }

impl Stack {
    fn push(mut self: Stack, v: i32): i32 { self.items.push(v); return 0 }
    fn pop(mut self: Stack): i32 { return self.items.pop() }
    fn top(self: Stack): i32 {
        let n: i32 = self.items.len()
        if n == 0 { return -1 }
        return self.items[n - 1]
    }
}
```

### Recursive types

```x
struct BranchData { v: i32, kids: Vec<Tree> }
enum Tree { Leaf, Branch(BranchData) }

fn sum_tree(t: Tree): i32 {
    match t {
        Leaf => { return 0 }
        Branch(d) => {
            let mut s: i32 = d.v
            let n: i32 = d.kids.len()
            let mut i: i32 = 0
            while i < n { s += sum_tree(d.kids[i]); i += 1 }
            return s
        }
    }
}
```

## Builtins (~130)

| Category | Builtins |
|----------|----------|
| Console | `print_i32 print_str print_raw print_f64` |
| Stderr | `eprint_str eprint_raw eprint_i32 eprint_f64 eprint_bool` |
| String | `str_len str_concat str_slice str_find str_find_from str_char_at str_lower str_upper str_replace str_repeat str_trim str_contains str_starts_with str_ends_with str_eq str_cmp str_translate str_delete str_keep str_translate_complement cat_show int_to_str float_to_str str_to_int str_to_float chr` |
| String builder | `sb_new sb_push sb_push_char sb_push_slice sb_push_i32 sb_str` |
| Regex (POSIX ERE) | `regex_match regex_find regex_find_from regex_match_len` (Linux; cached compilation) |
| Vec | `vec_new vec_len vec.push vec.pop vec.insert vec.remove_at` |
| File I/O | `read_file write_file read_stdin read_line sendfile_stdout count_newlines` |
| Filesystem | `remove_file make_dir chmod chown_file chgrp_file make_fifo mknod_dev file_exists file_size is_dir dir_count dir_entry stat_field statvfs_field cache_open cache_size` |
| Networking | `tcp_listen tcp_listen_reuseport tcp_connect accept recv_str recv_all send_str sendfile_fd sendfile_range close_fd epoll_create epoll_add epoll_del epoll_wait set_nonblock set_nodelay` |
| TLS/HTTPS | `tls_ctx_new tls_accept tls_read tls_write tls_close` (OpenSSL, gated) |
| Process | `fork getpid getuid getgid argc argv exit exec_split exec_sh kill sleep_sec wait_pid_status wait_child wait_status make_pipe pipe_read_end pipe_write_end` |
| Identity | `uid_to_name gid_to_name read_utmp` (getpwuid/getgrgid/getutent) |
| Time | `time_str time_format time_format_at time_format_at_utc time_now now_s fmt_ctime fmt_http_date` |
| Math | `abs max min int_to_f64 int_to_i64 pad_int pad_zero` |
| Self-check | `assert panic unreachable exit` |

## Coreutils (126) — Linux userland replacement

All written in xlang, compiled to C, cross-checked against GNU on Linux CI. Key tools:

- **grep**: `-r -n -i -v -c -H -o -l -A -B -C` + **POSIX ERE regex** (`grep -E`).
- **sed**: `s///[g]` (**ERE regex**) + `d p = q a i y c` commands, `-n`, `-i`, `-e`, `&` in replacement.
- **awk**: NR patterns, `/regex/` patterns, `BEGIN{}` / `END{}` blocks, `print $N/NR/NF`, `-F` separator.
- **find**: `-name -iname -type -maxdepth -exec CMD ; -size N[ckMG] -delete -mtime N -newer FILE`.
- **sort**: `-r -n -u -k F[,F]` (multi-key) + `-t DELIM`. **Faster than GNU sort** on Linux.
- **tr**: `-d -s -c` (complement). Bulk O(n) C builtins.
- **cut**: `-d -f -c`. Bulk delimiter scan via str_find_from.
- **date**: `+FORMAT -u -R -I -d @EPOCH -d 'YYYY-MM-DD' -d yesterday/tomorrow/'N days ago'`.
- **ls**: `-l -a -h -R`. Real user/group names via getpwuid/getgrgid.
- **stat**: `-c FORMAT` with `%n %s %F %f %h %u %U %g %G %y %a`.
- **df**: Real filesystem stats via statvfs. `-h` human-readable.
- **patch**: Unified-diff applicator (diff's companion).
- **xsh**: Shell with N-stage pipelines, redirects, `$VAR`, `$(cmd)`, `for/if/while`, `test/[`, `$((expr))`, `&&/||`.

Performance: most tools 1–2× GNU on Linux; several faster (sort 0.43×, rev 0.27×, wc 0.38×, uniq 0.89× faster than GNU).

## HTTP/HTTPS servers (14) — nginx replacement

| Server | Features |
|--------|----------|
| server_http | Full CRUD + ETag/304 + Last-Modified/304 + Range/206 + dir listing + CORS + dir redirect + **config-driven** (`-c conf`) + **multi-worker** (`-w N`, SO_REUSEPORT) + per-connection buffering + max request size (413) + sendfile |
| server_tls | **HTTPS** via OpenSSL FFI. Concurrent via `-w N` worker pool. |
| server_proxy | Reverse proxy with upstream keepalive + load balancing |
| server_vhost | Path-routing proxy (nginx location{} style) |
| server_gzip | **gzip_static** (serves pre-compressed .gz, binary-safe via sendfile) |
| server_web | **Multi-worker epoll** (`-w N`, SO_REUSEPORT). **Beats nginx per-core and multi-core.** |
| server_prefork, server_epoll, server_keepalive | Infrastructure variants |

Benchmarked against **nginx 1.28** (corrected xwrk): `server_web -w 16` ~199k req/s vs
nginx `worker_processes auto` ~180k (64 cores) — **xlang wins per-core and multi-core**
for keepalive file-serving.

## Testing

115 unit tests (lexer, parser, typecheck, codegen, source, error, driver, symbols, lsp).
All three repos CI-green on every PR. 6 benchmarks verify zero-overhead codegen
(xlang-generated code matches or beats hand-written C).

## Performance

xlang compiles to C → `gcc -O2`. Generated code matches hand-written C:

| Benchmark | xlang | Hand-written C |
|-----------|-------|----------------|
| Sieve (10M primes) | 0.06s | 0.06s |
| Popcount (2M) | 0.17s | 0.21s |
| String sort (4k) | 0.06s | 0.06s |
| Method calls (2M) | 0.06s | 0.06s |
| Enum VM (2M ops) | 0.07s | 0.06s |

## Methodology

Built iteratively: **replicate → hit a limitation → modify xlang → implement → verify**.
Each coreutil/server that needed a new capability drove xlang's growth.

## License

MIT
