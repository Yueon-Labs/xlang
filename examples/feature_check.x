module main

// Comprehensive feature check — exercises every major language feature
// in a single program. Run: xlangc run examples/feature_check.x

struct RectData {
    w: i32
    h: i32
}

enum Shape {
    Circle(f64)
    Rect(RectData)
}

struct Stack {
    items: Vec<i32>
}

impl Stack {
    fn push(mut self: Stack, v: i32): i32 {
        self.items.push(v)
        return 0
    }
    fn pop(mut self: Stack): i32 {
        return self.items.pop()
    }
    fn top(self: Stack): i32 {
        let n: i32 = self.items.len()
        if n == 0 { return -1 }
        return self.items[n - 1]
    }
}

fn half(n: i32): Option<i32> {
    if n % 2 == 0 { return Some(n / 2) }
    return None
}

fn main(): i32 {
    let s: String = "Hello, World"
    if s[0] == 'H' && s[7] == 'W' {
        print_str("string indexing + char literals: OK\n")
    }

    let n: i32 = -1
    match n {
        -1 => { print_str("negative match: OK\n") }
        0 => { print_str("zero\n") }
        _ => { print_str("other\n") }
    }
    let code: i32 = 42
    match code {
        0 | 1 => { print_str("low\n") }
        2..=99 => { print_str("range match: OK\n") }
        _ => { print_str("other\n") }
    }

    let shape: Shape = Circle(5.0)
    match shape {
        Circle(r) => {
            print_str("circle area = ")
            print_raw(float_to_str(3.14159 * r * r))
            print_str("\n")
        }
        Rect(d) => {
            print_str("rect area = ")
            print_i32(d.w * d.h)
            print_str("\n")
        }
    }

    let st: Stack = Stack { items: vec_new() }
    st.push(10)
    st.push(20)
    st.push(30)
    print_str("stack top = ")
    print_i32(st.top())
    print_str("\n")
    print_str("stack pop = ")
    print_i32(st.pop())
    print_str("\n")
    print_str("stack len = ")
    print_i32(st.items.len())
    print_str("\n")

    let v: Vec<i32> = vec_new()
    v.push(10)
    v.push(30)
    v.insert(1, 20)
    print_str("vec = ")
    let mut i: i32 = 0
    while i < v.len() {
        if i > 0 { print_str(",") }
        print_i32(v[i])
        i += 1
    }
    print_str("\n")

    let greeting: String = "x" + "lang"
    let line: String = "=" * 10
    print_str(greeting + " " + line + "\n")

    let mut x: i32 = 3
    let mut steps: i32 = 0
    while let Some(val) = half(x) {
        x = val
        steps += 1
    }
    print_str("while let steps = ")
    print_i32(steps)
    print_str("\n")

    let mask: i32 = 0xFF & 0x0F
    print_str("0xFF & 0x0F = ")
    print_i32(mask)
    print_str("\n")

    assert(mask == 15)
    assert(st.items.len() == 2)
    print_str("all features: OK\n")
    return 0
}
