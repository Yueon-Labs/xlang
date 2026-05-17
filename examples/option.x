module main

fn age_or_zero(age: Option<i32>): i32 {
    match age {
        Some(value) => {
            return value
        }
        None => {
            return 0
        }
    }
}
