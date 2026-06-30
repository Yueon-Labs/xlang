module main

// xsh — a shell: N-stage pipelines, redirects (<, >, >>), variable expansion
// ($VAR), export/assignment (NAME=VALUE), builtins (pwd/cd/echo/exit), and ';'.
//   a | b | c | d      N-stage pipeline   cmd > f / < f / >> f   redirects
//   echo $HOME         env-var expansion  export X=v / X=v        set shell var
//   cmd1 ; cmd2        sequence
// Uses exec_split (PATH-based) so PATH=xlang-bin runs a pure xlang userland.

fn is_name_start(c: i32): bool {
    if c >= 65 { if c <= 90 { return true } }
    if c >= 97 { if c <= 122 { return true } }
    if c == 95 { return true }
    return false
}

fn is_name_char(c: i32): bool {
    if is_name_start(c) { return true }
    if c >= 48 { if c <= 57 { return true } }
    return false
}

fn expand_vars(s: String): String {
    let n: i32 = str_len(s)
    sb_new()
    let mut i: i32 = 0
    while i < n {
        let c: i32 = str_char_at(s, i)
        if c == 36 {
            let j: i32 = i + 1
            if j < n {
                if is_name_start(str_char_at(s, j)) {
                    let mut k: i32 = j + 1
                    while k < n {
                        if is_name_char(str_char_at(s, k)) {
                            k = k + 1
                        } else {
                            break
                        }
                    }
                    sb_push(getenv(str_slice(s, j, k)))
                    i = k
                } else {
                    sb_push_char(36)
                    i = i + 1
                }
            } else {
                sb_push_char(36)
                i = i + 1
            }
        } else {
            sb_push_char(c)
            i = i + 1
        }
    }
    return sb_str()
}

// Run cmd in a child with stdout to a pipe; parent reads + returns raw output
// (no sb use, so callers that own the sb buffer are safe). Supports pipelines:
// the last stage's stdout is the (inherited) capture fd.
fn capture_cmd_raw(cmd: String): String {
    make_pipe()
    let r: i32 = pipe_read_end()
    let w: i32 = pipe_write_end()
    let pid: i32 = fork()
    if pid == 0 {
        close_fd(r)
        dup2(w, 1)
        close_fd(w)
        run_child(cmd)
        return ""
    }
    close_fd(w)
    let out: String = read_fd(r)
    close_fd(r)
    wait_child()
    return out
}

// Run a command (single or piped) in the current process's context — used by
// capture_cmd_raw's child so the LAST pipeline stage writes to the inherited
// stdout (the capture pipe).
fn run_child(cmd: String): i32 {
    let stages: Vec<String> = split_pipe(cmd)
    if vec_len(stages) > 1 {
        run_pipeline(stages)
    } else {
        exec_split(cmd)
    }
    return 0
}

// Combined expansion in ONE pass (one sb): $(cmd) command substitution AND
// $VAR. Doing both in one scan avoids the two-sb-pass conflict (each pass
// resets the global sb buffer).
fn expand(s: String): String {
    let n: i32 = str_len(s)
    sb_new()
    let mut i: i32 = 0
    while i < n {
        let c: i32 = str_char_at(s, i)
        if c == 36 {
            if i + 1 < n {
                if str_char_at(s, i + 1) == 40 {
                    let mut depth: i32 = 1
                    let mut j: i32 = i + 2
                    while j < n {
                        let cc: i32 = str_char_at(s, j)
                        if cc == 40 {
                            depth = depth + 1
                        }
                        if cc == 41 {
                            depth = depth - 1
                            if depth == 0 {
                                break
                            }
                        }
                        j = j + 1
                    }
                    if depth == 0 {
                        let out: String = capture_cmd_raw(str_slice(s, i + 2, j))
                        let on: i32 = str_len(out)
                        let mut oend: i32 = on
                        if oend > 0 {
                            if str_char_at(out, oend - 1) == 10 {
                                oend = oend - 1
                            }
                        }
                        let mut k: i32 = 0
                        while k < oend {
                            let oc: i32 = str_char_at(out, k)
                            if oc == 10 {
                                sb_push_char(32)
                            } else {
                                sb_push_char(oc)
                            }
                            k = k + 1
                        }
                        i = j + 1
                    } else {
                        sb_push_char(36)
                        i = i + 1
                    }
                } else {
                    if is_name_start(str_char_at(s, i + 1)) {
                        let mut k: i32 = i + 2
                        while k < n {
                            if is_name_char(str_char_at(s, k)) {
                                k = k + 1
                            } else {
                                break
                            }
                        }
                        sb_push(getenv(str_slice(s, i + 1, k)))
                        i = k
                    } else {
                        sb_push_char(36)
                        i = i + 1
                    }
                }
            } else {
                sb_push_char(36)
                i = i + 1
            }
        } else {
            sb_push_char(c)
            i = i + 1
        }
    }
    return sb_str()
}

fn split_char(s: String, sep: i32): Vec<String> {
    let v: Vec<String> = vec_new()
    let n: i32 = str_len(s)
    let mut start: i32 = 0
    let mut i: i32 = 0
    while i < n {
        if str_char_at(s, i) == sep {
            v.push(str_slice(s, start, i))
            start = i + 1
        }
        i = i + 1
    }
    v.push(str_slice(s, start, n))
    return v
}

fn split_pipe(s: String): Vec<String> {
    let v: Vec<String> = vec_new()
    let n: i32 = str_len(s)
    let mut start: i32 = 0
    let mut i: i32 = 0
    while i < n {
        if i + 2 < n {
            if str_char_at(s, i) == 32 {
                if str_char_at(s, i + 1) == 124 {
                    if str_char_at(s, i + 2) == 32 {
                        v.push(str_slice(s, start, i))
                        start = i + 3
                        i = i + 3
                    } else {
                        i = i + 1
                    }
                } else {
                    i = i + 1
                }
            } else {
                i = i + 1
            }
        } else {
            i = i + 1
        }
    }
    v.push(str_slice(s, start, n))
    return v
}

fn close_all_pipes(npipes: i32): i32 {
    let mut j: i32 = 0
    while j < npipes {
        close_fd(pipe_r_at(j))
        close_fd(pipe_w_at(j))
        j = j + 1
    }
    return 0
}

fn run_pipeline(stages: Vec<String>): i32 {
    let ns: i32 = vec_len(stages)
    let npipes: i32 = ns - 1
    let mut i: i32 = 0
    while i < npipes {
        make_pipe_at(i)
        i = i + 1
    }
    let mut k: i32 = 0
    while k < ns {
        let pid: i32 = fork()
        if pid == 0 {
            if k > 0 {
                dup2(pipe_r_at(k - 1), 0)
            }
            if k < ns - 1 {
                dup2(pipe_w_at(k), 1)
            }
            close_all_pipes(npipes)
            exec_split(stages[k])
            return 1
        }
        k = k + 1
    }
    close_all_pipes(npipes)
    let mut w: i32 = 0
    while w < ns {
        wait_child()
        w = w + 1
    }
    return 0
}

fn run_one(cmd: String): i32 {
    let mut core: String = cmd
    let mut infile: String = ""
    let mut outfile: String = ""
    let mut append: bool = false
    let ai: i32 = str_find(core, " >> ")
    if ai >= 0 {
        outfile = str_slice(core, ai + 4, str_len(core))
        core = str_slice(core, 0, ai)
        append = true
    } else {
        let oi: i32 = str_find(core, " > ")
        if oi >= 0 {
            outfile = str_slice(core, oi + 3, str_len(core))
            core = str_slice(core, 0, oi)
        }
    }
    let ii: i32 = str_find(core, " < ")
    if ii >= 0 {
        infile = str_slice(core, ii + 3, str_len(core))
        core = str_slice(core, 0, ii)
    }
    let pid: i32 = fork()
    if pid == 0 {
        if str_len(infile) > 0 {
            let fdi: i32 = open_read(infile)
            dup2(fdi, 0)
            close_fd(fdi)
        }
        if str_len(outfile) > 0 {
            let mut fdo: i32 = 0
            if append {
                fdo = open_append(outfile)
            } else {
                fdo = open_write(outfile)
            }
            dup2(fdo, 1)
            close_fd(fdo)
        }
        exec_split(core)
        return 1
    }
    wait_child()
    return 0
}

fn trim(s: String): String {
    let n: i32 = str_len(s)
    let mut start: i32 = 0
    while start < n {
        if str_char_at(s, start) == 32 {
            start = start + 1
        } else {
            break
        }
    }
    let mut end: i32 = n
    while end > start {
        if str_char_at(s, end - 1) == 32 {
            end = end - 1
        } else {
            break
        }
    }
    return str_slice(s, start, end)
}

fn strip_trailing_semi(s: String): String {
    let t: String = trim(s)
    let n: i32 = str_len(t)
    if n > 0 {
        if str_char_at(t, n - 1) == 59 {
            return str_slice(t, 0, n - 1)
        }
    }
    return t
}

// for VAR in w1 w2 ...; do BODY; done  (single-line)
fn run_for(c: String): i32 {
    let do_pos: i32 = str_find(c, " do ")
    if do_pos < 0 {
        return 1
    }
    let varlist: String = str_slice(c, 4, do_pos)
    let body_full: String = str_slice(c, do_pos + 4, str_len(c))
    let done_pos: i32 = str_find(body_full, "done")
    if done_pos < 0 {
        return 1
    }
    let body: String = strip_trailing_semi(str_slice(body_full, 0, done_pos))
    let in_pos: i32 = str_find(varlist, " in ")
    if in_pos < 0 {
        return 1
    }
    let var: String = str_slice(varlist, 0, in_pos)
    let raw_list: String = strip_trailing_semi(str_slice(varlist, in_pos + 4, str_len(varlist)))
    let list_str: String = expand(raw_list)
    let words: Vec<String> = split_char(list_str, 32)
    let mut i: i32 = 0
    let wc: i32 = vec_len(words)
    while i < wc {
        let w: String = trim(words[i])
        if str_len(w) > 0 {
            setenv(var, w)
            run_command(body)
        }
        i = i + 1
    }
    return 0
}

fn run_command(cmd: String): i32 {
    // for-loop must be handled on the RAW line (before $VAR expansion), so the
    // body's $VAR is re-expanded each iteration with the loop variable set.
    if str_find(cmd, "for ") == 0 {
        if str_find(cmd, " do ") >= 0 {
            return run_for(cmd)
        }
    }
    let c: String = trim(expand(cmd))
    if str_len(c) == 0 {
        return 0
    }
    // Pipes / redirects take precedence over builtins, so `echo x | grep` pipes
    // and `echo x > f` redirects (rather than hitting the echo builtin).
    let stages: Vec<String> = split_pipe(c)
    if vec_len(stages) > 1 {
        run_pipeline(stages)
        return 0
    }
    if str_find(c, " > ") >= 0 {
        run_one(c)
        return 0
    }
    if str_find(c, " >> ") >= 0 {
        run_one(c)
        return 0
    }
    if str_find(c, " < ") >= 0 {
        run_one(c)
        return 0
    }
    // Builtins (standalone commands only).
    if str_eq(c, "pwd") {
        print_raw(getcwd())
        print_raw("\n")
        return 0
    }
    if str_eq(c, "exit") {
        return -1
    }
    if str_find(c, "cd ") == 0 {
        chdir(str_slice(c, 3, str_len(c)))
        return 0
    }
    if str_find(c, "echo ") == 0 {
        print_raw(str_slice(c, 5, str_len(c)))
        print_raw("\n")
        return 0
    }
    if str_eq(c, "echo") {
        print_raw("\n")
        return 0
    }
    if str_find(c, "export ") == 0 {
        let body: String = str_slice(c, 7, str_len(c))
        let eq: i32 = str_find(body, "=")
        if eq > 0 {
            setenv(str_slice(body, 0, eq), str_slice(body, eq + 1, str_len(body)))
        }
        return 0
    }
    let eq0: i32 = str_find(c, "=")
    if eq0 > 0 {
        let sp: i32 = str_find(c, " ")
        if sp < 0 {
            setenv(str_slice(c, 0, eq0), str_slice(c, eq0 + 1, str_len(c)))
            return 0
        }
    }
    run_one(c)
    return 0
}

fn main(): i32 {
    while true {
        let cmd: String = read_line()
        if str_len(cmd) == 0 {
            return 0
        }
        // A for-loop line contains ';' as syntax (for ...; do ...; done), so it
        // must bypass the ';' command-split and go straight to run_for.
        if str_find(cmd, "for ") == 0 {
            if str_find(cmd, " do ") >= 0 {
                run_for(cmd)
                continue
            }
        }
        // Split raw line on ';'. Variable expansion is per-command (deferred to
        // run_command), so `X=hi; echo $X` sees X set before $X expands.
        let cmds: Vec<String> = split_char(cmd, 59)
        let mut c: i32 = 0
        let mut stop: bool = false
        while c < vec_len(cmds) {
            if run_command(cmds[c]) < 0 {
                stop = true
            }
            c = c + 1
            if stop {
                return 0
            }
        }
    }
    return 0
}
