module main

// rpn — a reverse-Polish-notation calculator.
// Demonstrates: Vec push/pop (the complete Vec API), char literals ('+' etc),
// string parsing (tokenize by spaces), str_to_int/str_slice, arithmetic.
//
// Usage: xlangc run examples/rpn.x "3 4 + 5 *"   → 35
//        xlangc run examples/rpn.x "1 2 3 + +"    → 6

fn main(): i32 {
    if argc() < 2 {
        print_str("usage: rpn <expr>\n")
        return 1
    }
    let expr: String = argv(1)
    let stack: Vec<i32> = vec_new()
    let n: i32 = str_len(expr)
    let mut i: i32 = 0
    while i < n {
        // skip spaces
        while i < n {
            if str_char_at(expr, i) == ' ' { i += 1 } else { break }
        }
        if i >= n { break }
        let c: i32 = str_char_at(expr, i)
        // Is it a single-char operator? (followed by space or EOF — so "-4" is a
        // number, but "3 4 -" has '-' as subtraction)
        let next_is_sep: bool = (i + 1 >= n) || (str_char_at(expr, i + 1) == ' ')
        if next_is_sep {
            if c == '+' {
                let b: i32 = stack.pop()
                let a: i32 = stack.pop()
                stack.push(a + b)
                i += 1
            } else if c == '-' {
                let b: i32 = stack.pop()
                let a: i32 = stack.pop()
                stack.push(a - b)
                i += 1
            } else if c == '*' {
                let b: i32 = stack.pop()
                let a: i32 = stack.pop()
                stack.push(a * b)
                i += 1
            } else if c == '/' {
                let b: i32 = stack.pop()
                let a: i32 = stack.pop()
                stack.push(a / b)
                i += 1
            } else {
                // not an operator — parse as number
                let start: i32 = i
                while i < n {
                    if str_char_at(expr, i) == ' ' { break }
                    i += 1
                }
                stack.push(str_to_int(str_slice(expr, start, i)))
            }
        } else {
            // multi-char token → number
            let start: i32 = i
            while i < n {
                if str_char_at(expr, i) == ' ' { break }
                i += 1
            }
            stack.push(str_to_int(str_slice(expr, start, i)))
        }
    }
    print_i32(stack.pop())
    return 0
}
