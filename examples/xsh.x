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

// Arithmetic for $((expr)): recursive descent (+,-,*,/,parens,unary-,numbers,
// and bare/$-prefixed variable names → getenv). The position cursor is a
// Vec<i32> of length 1, mutated via index assignment — shared across the
// recursive calls (Vec data lives on the heap), so callees advance it.
fn arith_skip(e: String, pos: Vec<i32>): i32 {
    let n: i32 = str_len(e)
    while pos[0] < n {
        if str_char_at(e, pos[0]) == 32 {
            pos[0] = pos[0] + 1
        } else {
            break
        }
    }
    return 0
}

fn arith_factor(e: String, pos: Vec<i32>): i32 {
    let n: i32 = str_len(e)
    arith_skip(e, pos)
    if pos[0] >= n {
        return 0
    }
    let c: i32 = str_char_at(e, pos[0])
    if c == 45 {
        pos[0] = pos[0] + 1
        return 0 - arith_factor(e, pos)
    }
    if c == 40 {
        pos[0] = pos[0] + 1
        let v: i32 = arith_expr(e, pos)
        arith_skip(e, pos)
        if pos[0] < n {
            if str_char_at(e, pos[0]) == 41 {
                pos[0] = pos[0] + 1
            }
        }
        return v
    }
    if c == 36 {
        pos[0] = pos[0] + 1
    }
    if is_name_start(str_char_at(e, pos[0])) {
        let name_start: i32 = pos[0]
        while pos[0] < n {
            if is_name_char(str_char_at(e, pos[0])) {
                pos[0] = pos[0] + 1
            } else {
                break
            }
        }
        return str_to_int(getenv(str_slice(e, name_start, pos[0])))
    }
    let mut v: i32 = 0
    while pos[0] < n {
        let d: i32 = str_char_at(e, pos[0])
        if d >= 48 {
            if d <= 57 {
                v = v * 10 + (d - 48)
                pos[0] = pos[0] + 1
            } else {
                break
            }
        } else {
            break
        }
    }
    return v
}

fn arith_term(e: String, pos: Vec<i32>): i32 {
    let n: i32 = str_len(e)
    let mut v: i32 = arith_factor(e, pos)
    while true {
        arith_skip(e, pos)
        if pos[0] >= n {
            break
        }
        let c: i32 = str_char_at(e, pos[0])
        if c == 42 {
            pos[0] = pos[0] + 1
            v = v * arith_factor(e, pos)
        } else {
            if c == 47 {
                pos[0] = pos[0] + 1
                v = v / arith_factor(e, pos)
            } else {
                break
            }
        }
    }
    return v
}

fn arith_expr(e: String, pos: Vec<i32>): i32 {
    let n: i32 = str_len(e)
    let mut v: i32 = arith_term(e, pos)
    while true {
        arith_skip(e, pos)
        if pos[0] >= n {
            break
        }
        let c: i32 = str_char_at(e, pos[0])
        if c == 43 {
            pos[0] = pos[0] + 1
            v = v + arith_term(e, pos)
        } else {
            if c == 45 {
                pos[0] = pos[0] + 1
                v = v - arith_term(e, pos)
            } else {
                break
            }
        }
    }
    return v
}

fn eval_arith(e: String): i32 {
    let pos: Vec<i32> = vec_new()
    pos.push(0)
    return arith_expr(e, pos)
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
            // $((expr)) arithmetic substitution (checked before $(cmd))
            if i + 2 < n {
                if str_char_at(s, i + 1) == 40 {
                    if str_char_at(s, i + 2) == 40 {
                        let mut adep: i32 = 0
                        let mut j: i32 = i + 3
                        while j < n {
                            let cc: i32 = str_char_at(s, j)
                            if cc == 40 {
                                adep = adep + 1
                            }
                            if cc == 41 {
                                if adep == 0 {
                                    break
                                }
                                adep = adep - 1
                            }
                            j = j + 1
                        }
                        if j < n {
                            if str_char_at(s, j) == 41 {
                                sb_push(int_to_str(eval_arith(str_slice(s, i + 3, j))))
                                i = j + 2
                                continue
                            }
                        }
                    }
                }
            }
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
    let mut code: i32 = 0
    while w < ns {
        code = wait_status()
        w = w + 1
    }
    return code
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
    let code: i32 = wait_status()
    return code
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
            run_body(body)
        }
        i = i + 1
    }
    return 0
}

// if COND; then BODY; fi  (single-line). Runs BODY iff COND exits 0.
fn run_if(c: String): i32 {
    let then_pos: i32 = str_find(c, " then ")
    if then_pos < 0 {
        return 1
    }
    let cond: String = strip_trailing_semi(str_slice(c, 3, then_pos))
    let body_full: String = str_slice(c, then_pos + 6, str_len(c))
    let fi_pos: i32 = str_find(body_full, "fi")
    if fi_pos < 0 {
        return 1
    }
    let body: String = strip_trailing_semi(str_slice(body_full, 0, fi_pos))
    if run_command(cond) == 0 {
        run_body(body)
    }
    return 0
}

// while COND; do BODY; done  (single-line). Runs BODY while COND exits 0.
fn run_while(c: String): i32 {
    let do_pos: i32 = str_find(c, " do ")
    if do_pos < 0 {
        return 1
    }
    let cond: String = strip_trailing_semi(str_slice(c, 6, do_pos))
    let body_full: String = str_slice(c, do_pos + 4, str_len(c))
    let done_pos: i32 = str_find(body_full, "done")
    if done_pos < 0 {
        return 1
    }
    let body: String = strip_trailing_semi(str_slice(body_full, 0, done_pos))
    while true {
        if run_command(cond) != 0 {
            break
        }
        run_body(body)
    }
    return 0
}

// Run a control-flow BODY, which may contain ';'-separated commands. (Nested
// control flow inside a body isn't supported — the ';' split would break it.)
fn run_body(body: String): i32 {
    let parts: Vec<String> = split_char(body, 59)
    let mut k: i32 = 0
    while k < vec_len(parts) {
        run_command(parts[k])
        k = k + 1
    }
    return 0
}

// test / [ ... ] — evaluate a condition, return 0 (true) or 1 (false).
// Forms: -f/-d/-e/-z/-n ARG  (1 op),  A -lt/-gt/-eq/-ne/-le/-ge/= /!= B  (2 op).
fn run_test(s: String): i32 {
    let toks: Vec<String> = split_char(trim(s), 32)
    let mut args: Vec<String> = vec_new()
    let mut t: i32 = 0
    while t < vec_len(toks) {
        let tk: String = trim(toks[t])
        if str_len(tk) > 0 {
            args.push(tk)
        }
        t = t + 1
    }
    let n: i32 = vec_len(args)
    if n == 2 {
        let op: String = args[0]
        let a: String = args[1]
        if str_eq(op, "-f") {
            if file_exists(a) {
                return 0
            }
            return 1
        }
        if str_eq(op, "-d") {
            if is_dir(a) {
                return 0
            }
            return 1
        }
        if str_eq(op, "-e") {
            if file_exists(a) {
                return 0
            }
            return 1
        }
        if str_eq(op, "-z") {
            if str_len(a) == 0 {
                return 0
            }
            return 1
        }
        if str_eq(op, "-n") {
            if str_len(a) > 0 {
                return 0
            }
            return 1
        }
        return 1
    }
    if n == 3 {
        let a: String = args[0]
        let op: String = args[1]
        let b: String = args[2]
        if str_eq(op, "-lt") {
            if str_to_int(a) < str_to_int(b) {
                return 0
            }
            return 1
        }
        if str_eq(op, "-gt") {
            if str_to_int(a) > str_to_int(b) {
                return 0
            }
            return 1
        }
        if str_eq(op, "-le") {
            if str_to_int(a) <= str_to_int(b) {
                return 0
            }
            return 1
        }
        if str_eq(op, "-ge") {
            if str_to_int(a) >= str_to_int(b) {
                return 0
            }
            return 1
        }
        if str_eq(op, "-eq") {
            if str_to_int(a) == str_to_int(b) {
                return 0
            }
            return 1
        }
        if str_eq(op, "-ne") {
            if str_to_int(a) != str_to_int(b) {
                return 0
            }
            return 1
        }
        if str_eq(op, "=") {
            if str_eq(a, b) {
                return 0
            }
            return 1
        }
        if str_eq(op, "==") {
            if str_eq(a, b) {
                return 0
            }
            return 1
        }
        if str_eq(op, "!=") {
            if !str_eq(a, b) {
                return 0
            }
            return 1
        }
        return 1
    }
    return 1
}

fn run_command(cmd: String): i32 {
    // Control-flow constructs are handled on the RAW line (before $VAR
    // expansion), so their bodies re-expand per execution.
    if str_find(cmd, "for ") == 0 {
        if str_find(cmd, " do ") >= 0 {
            return run_for(cmd)
        }
    }
    if str_find(cmd, "if ") == 0 {
        if str_find(cmd, " then ") >= 0 {
            return run_if(cmd)
        }
    }
    if str_find(cmd, "while ") == 0 {
        if str_find(cmd, " do ") >= 0 {
            return run_while(cmd)
        }
    }
    // Function definition: name() { body }  → store in env (before expand).
    if str_find(cmd, "() {") >= 0 {
        let paren: i32 = str_find(cmd, "() {")
        let name: String = trim(str_slice(cmd, 0, paren))
        let brace: i32 = str_find(cmd, "{")
        let body: String = trim(str_slice(cmd, brace + 1, str_len(cmd) - 1))
        if str_len(name) > 0 {
            setenv(str_concat("__xlang_func_", name), body)
        }
        return 0
    }
    let c: String = trim(expand(cmd))
    if str_len(c) == 0 {
        return 0
    }
    // Function call: if the first word matches a defined function, run its body.
    let mut sp: i32 = str_find(c, " ")
    if sp < 0 {
        sp = str_len(c)
    }
    let fw: String = str_slice(c, 0, sp)
    let fbody: String = getenv(str_concat("__xlang_func_", fw))
    if str_len(fbody) > 0 {
        return run_body(fbody)
    }
    // Compound commands: split at the FIRST `&&` or `||`, short-circuit.
    // (left has no operator after the first split, so recursion terminates.)
    let and_pos: i32 = str_find(c, " && ")
    let or_pos: i32 = str_find(c, " || ")
    let mut split_pos: i32 = -1
    let mut op: String = ""
    if and_pos >= 0 {
        split_pos = and_pos
        op = "&&"
    }
    if or_pos >= 0 {
        if split_pos < 0 || or_pos < split_pos {
            split_pos = or_pos
            op = "||"
        }
    }
    if split_pos >= 0 {
        let left: String = trim(str_slice(c, 0, split_pos))
        let right: String = trim(str_slice(c, split_pos + 4, str_len(c)))
        let lc: i32 = run_command(left)
        if str_eq(op, "&&") {
            if lc == 0 {
                return run_command(right)
            }
            return lc
        }
        if lc != 0 {
            return run_command(right)
        }
        return lc
    }
    // test / [ ... ] — returns 0 (true) or 1 (false); checked before pipes so
    // `[ a -lt b ]` is a condition, not a redirect.
    if str_char_at(c, 0) == 91 {
        let close: i32 = str_find(c, "]")
        if close > 0 {
            return run_test(str_slice(c, 1, close))
        }
    }
    if str_find(c, "test ") == 0 {
        return run_test(str_slice(c, 5, str_len(c)))
    }
    // Pipes / redirects take precedence over builtins, so `echo x | grep` pipes
    // and `echo x > f` redirects (rather than hitting the echo builtin).
    let stages: Vec<String> = split_pipe(c)
    if vec_len(stages) > 1 {
        return run_pipeline(stages)
    }
    if str_find(c, " > ") >= 0 {
        return run_one(c)
    }
    if str_find(c, " >> ") >= 0 {
        return run_one(c)
    }
    if str_find(c, " < ") >= 0 {
        return run_one(c)
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
    if str_eq(c, "true") {
        return 0
    }
    if str_eq(c, "false") {
        return 1
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
    return run_one(c)
}

fn main(): i32 {
    while true {
        let cmd: String = read_line()
        if str_len(cmd) == 0 {
            return 0
        }
        // Control-flow lines contain ';' as syntax (for/if/while ...; do/then
        // ...; done/fi), so they bypass the ';' command-split.
        if str_find(cmd, "for ") == 0 {
            if str_find(cmd, " do ") >= 0 {
                run_for(cmd)
                continue
            }
        }
        if str_find(cmd, "if ") == 0 {
            if str_find(cmd, " then ") >= 0 {
                run_if(cmd)
                continue
            }
        }
        if str_find(cmd, "while ") == 0 {
            if str_find(cmd, " do ") >= 0 {
                run_while(cmd)
                continue
            }
        }
        // Function definitions contain ';' inside {} — bypass the ';' split.
        if str_find(cmd, "() {") >= 0 {
            run_command(cmd)
            continue
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
