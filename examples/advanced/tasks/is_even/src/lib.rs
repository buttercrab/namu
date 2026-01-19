use namu::prelude::*;

#[task(single)]
pub fn is_even(value: i32) -> Result<bool> {
    Ok(value % 2 == 0)
}

register_task! {
    method = is_even,
    name = "is_even",
    author = "Namu",
    version = "0.1.0"
}
