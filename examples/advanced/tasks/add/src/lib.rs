use namu::prelude::*;

#[task(single)]
pub fn add(a: i32, b: i32) -> Result<i32> {
    Ok(a + b)
}

register_task! {
    method = add,
    name = "add",
    author = "Namu",
    version = "0.1.0"
}
