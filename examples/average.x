module main

fn average(values: Slice<f64>): Result<f64, String> {
    if values.len() == 0 {
        return Err("empty array")
    }

    let mut sum: f64 = 0.0

    for value in values {
        sum = sum + value
    }

    return Ok(sum / values.len().to_f64())
}
