module main

// stats — a struct holding a Vec<i32>, with read-only methods. Demonstrates
// that structs with collection fields now compile (the typedef-ordering fix),
// and that those fields are usable via vec_len + indexing inside methods.
// Run: xlangc run examples/stats.x

struct Stats {
    data: Vec<i32>
}

impl Stats {
    fn count(self: Stats): i32 {
        return vec_len(self.data)
    }
    fn sum(self: Stats): i32 {
        let mut total: i32 = 0
        let n: i32 = vec_len(self.data)
        let mut i: i32 = 0
        while i < n {
            total += self.data[i]
            i += 1
        }
        return total
    }
    fn max(self: Stats): i32 {
        let n: i32 = vec_len(self.data)
        let mut m: i32 = self.data[0]
        let mut i: i32 = 1
        while i < n {
            if self.data[i] > m { m = self.data[i] }
            i += 1
        }
        return m
    }
}

fn main(): i32 {
    let mut buf: Vec<i32> = vec_new()
    buf.push(3)
    buf.push(9)
    buf.push(5)
    buf.push(7)
    let s: Stats = Stats { data: buf }
    print_i32(s.count())   // 4
    print_i32(s.sum())     // 24
    print_i32(s.max())     // 9
    assert(s.sum() == 24)
    assert(s.max() == 9)
    return 0
}
