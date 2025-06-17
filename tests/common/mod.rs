//! Common test utilities, tasks, and setup for graph integration tests.

use anyhow::Result;
use namu::task;

// --- Test Tasks ---

#[task]
pub fn add(a: i32, b: i32) -> Result<i32> {
    Ok(a + b)
}

#[task]
pub fn is_positive(v: i32) -> Result<bool> {
    Ok(v > 0)
}

#[task]
pub fn double(v: i32) -> Result<i32> {
    Ok(v * 2)
}

#[task]
pub fn identity(v: i32) -> Result<i32> {
    Ok(v)
}

#[task]
pub fn is_negative(v: i32) -> Result<bool> {
    Ok(v < 0)
}

#[task]
pub fn less_than(a: i32, b: i32) -> Result<bool> {
    Ok(a < b)
}

#[task]
pub fn is_even(n: i32) -> Result<bool> {
    Ok(n % 2 == 0)
}

#[task]
pub fn divide_by_2(n: i32) -> Result<i32> {
    Ok(n / 2)
}

#[task]
pub fn multiply_by_3_and_add_1(n: i32) -> Result<i32> {
    Ok(n * 3 + 1)
}

#[task]
pub fn not_one(n: i32) -> Result<bool> {
    Ok(n != 1)
}

#[task]
#[allow(unreachable_code)]
pub fn panicker() -> Result<i32> {
    panic!("This should not be called!");
}
