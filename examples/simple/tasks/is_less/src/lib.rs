use namu::prelude::*;

#[task]
pub fn is_less(a: i32, b: i32) -> Result<bool> {
    Ok(a < b)
}
