use namu::prelude::*;

#[task(single)]
pub fn is_less(a: i32, b: i32) -> Result<bool> {
    Ok(a < b)
}

register_task! {
    method = is_less,
    name = "is_less",
    author = "Jaeyong Sung",
    version = "0.1"
}
