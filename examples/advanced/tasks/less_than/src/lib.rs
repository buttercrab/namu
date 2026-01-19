use namu::prelude::*;

#[task(single)]
pub fn less_than(a: i32, b: i32) -> Result<bool> {
    Ok(a < b)
}

register_task! {
    method = less_than,
    name = "less_than",
    author = "Namu",
    version = "0.1.0"
}
