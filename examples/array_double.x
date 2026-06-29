module main

fn main(): i32 {
    let mut nums: Array<i32, 4> = [1, 2, 3, 4]
    let mut i: i32 = 0
    while i < 4 {
        nums[i] = nums[i] * 2
        i += 1
    }
    let mut sum: i32 = 0
    let mut j: i32 = 0
    while j < 4 {
        sum += nums[j]
        j += 1
    }
    print_i32(sum)
    return sum
}
