use namu::prelude::*;

#[task]
pub fn add(a: i32, b: i32) -> Result<i32> {
    Ok(a + b)
}
