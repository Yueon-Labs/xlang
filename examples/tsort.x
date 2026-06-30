module main

// tsort [file] — topological sort (like GNU tsort).
// Input: pairs "a b" meaning a must come before b. One-per-line or pair-per-line.
// Output: one node per line in topological order.

fn find_or_add(nodes: Vec<String>, name: String): i32 {
    let mut i: i32 = 0
    while i < vec_len(nodes) {
        if str_eq(nodes[i], name) {
            return i
        }
        i = i + 1
    }
    return -1
}

fn main(): i32 {
    let mut s: String = ""
    if argc() >= 2 {
        s = read_file(argv(1))
    } else {
        s = read_stdin()
    }
    let n: i32 = str_len(s)
    let nodes: Vec<String> = vec_new()
    let indeg: Vec<i32> = vec_new()
    let edges: Vec<String> = vec_new()
    let order: Vec<i32> = vec_new()
    let mut i: i32 = 0
    let mut start: i32 = 0
    let mut tokens: Vec<String> = vec_new()
    while i <= n {
        let at_nl: bool = i < n && str_char_at(s, i) == 10
        let at_end: bool = i == n && start < n
        if !at_nl && !at_end {
            i = i + 1
            continue
        }
        let raw: String = str_slice(s, start, i)
        let mut bstart: i32 = 0
        let mut bend: i32 = str_len(raw)
        while bstart < bend {
            if str_char_at(raw, bstart) == 32 {
                bstart = bstart + 1
            } else {
                break
            }
        }
        while bend > bstart {
            if str_char_at(raw, bend - 1) == 32 {
                bend = bend - 1
            } else {
                break
            }
        }
        let line: String = str_slice(raw, bstart, bend)
        if str_len(line) > 0 {
            let mut ws: i32 = str_find(line, " ")
            if ws < 0 {
                ws = str_find(line, "\t")
            }
            let mut a: String = line
            let mut b: String = line
            if ws >= 0 {
                a = str_slice(line, 0, ws)
                b = str_slice(line, ws + 1, str_len(line))
            }
            tokens.push(a)
            tokens.push(b)
        }
        start = i + 1
        i = i + 1
    }
    let tcount: i32 = vec_len(tokens)
    let mut ti: i32 = 0
    while ti < tcount {
        let name: String = tokens[ti]
        let idx: i32 = find_or_add(nodes, name)
        if idx < 0 {
            nodes.push(name)
            indeg.push(0)
            edges.push("")
        }
        ti = ti + 1
    }
    let node_count: i32 = vec_len(nodes)
    ti = 0
    while ti < tcount {
        let a: String = tokens[ti]
        let b: String = tokens[ti + 1]
        if !str_eq(a, b) {
            let bi: i32 = find_or_add(nodes, b)
            indeg[bi] = indeg[bi] + 1
        }
        ti = ti + 2
    }
    let mut changed: bool = true
    while changed {
        changed = false
        let mut ni: i32 = 0
        while ni < node_count {
            if indeg[ni] == 0 {
                let mut ok: bool = true
                let mut oi: i32 = 0
                while oi < vec_len(order) {
                    if order[oi] == ni {
                        ok = false
                    }
                    oi = oi + 1
                }
                if ok {
                    order.push(ni)
                    changed = true
                    ti = 0
                    while ti < tcount {
                        if str_eq(tokens[ti], nodes[ni]) {
                            let bi: i32 = find_or_add(nodes, tokens[ti + 1])
                            if bi >= 0 {
                                indeg[bi] = indeg[bi] - 1
                            }
                        }
                        ti = ti + 2
                    }
                }
            }
            ni = ni + 1
        }
    }
    let mut oi: i32 = 0
    while oi < vec_len(order) {
        print_raw(nodes[order[oi]])
        print_raw("\n")
        oi = oi + 1
    }
    return 0
}
