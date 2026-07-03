module main

// stack_demo — a mutable Stack (Vec<i32> field + mut self methods).
// Demonstrates: mut self (push/pop persist through pointer), push-on-field
// (self.items.push), by-value read-only methods (peek/size/sum), and the
// interplay between the two.
// Run: xlangc run examples/stack_demo.x

struct Stack {
    items: Vec<i32>
    len: i32
}

impl Stack {
    // mut self: mutations persist to the caller's object.
    fn push(mut self: Stack, v: i32): i32 {
        self.items.push(v)
        self.len += 1
        return 0
    }
    fn pop(mut self: Stack): i32 {
        if self.len == 0 { return -1 }
        self.len -= 1
        return self.items[self.len]
    }
    // by-value self: read-only, no mutation needed.
    fn peek(self: Stack): i32 {
        if self.len == 0 { return -1 }
        return self.items[self.len - 1]
    }
    fn size(self: Stack): i32 {
        return self.len
    }
    fn sum(self: Stack): i32 {
        let mut total: i32 = 0
        let mut i: i32 = 0
        while i < self.len {
            total += self.items[i]
            i += 1
        }
        return total
    }
}

fn main(): i32 {
    let mut buf: Vec<i32> = vec_new()
    let s: Stack = Stack { items: buf, len: 0 }
    // Push (mut self — persists via pointer)
    s.push(10)
    s.push(20)
    s.push(30)
    print_str("peek: ")
    print_i32(s.peek())
    print_str("\nsize: ")
    print_i32(s.size())
    print_str("\nsum: ")
    print_i32(s.sum())
    // Pop (mut self — decrements len)
    print_str("\npop: ")
    print_i32(s.pop())
    print_str("\npeek after pop: ")
    print_i32(s.peek())
    print_str("\nsize after pop: ")
    print_i32(s.size())
    print_str("\n")
    assert(s.size() == 2)
    assert(s.peek() == 20)
    return 0
}
