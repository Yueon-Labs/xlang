module main

struct User {
    id: i64
    name: String
    age: Option<i32>
}

fn is_adult(age: i32): bool {
    return age >= 18
}
