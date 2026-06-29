module main

fn main(): i32 {
    let nums: Array<i32, 5> = [10, 20, 30, 40, 50]
    let mut sum: i32 = 0
    for n in nums {
        sum += n
    }
    print_i32(sum)
    return sum
}
