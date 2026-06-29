module main

struct Point {
    x: i32
    y: i32
}

fn make_point(a: i32, b: i32): Point {
    return Point { x: a, y: b }
}

fn get_x(p: Point): i32 {
    return p.x
}

fn main(): i32 {
    let p: Point = Point { x: 3, y: 4 }
    let q: Point = make_point(10, 20)
    return p.x + p.y + get_x(q)
}
