use namu::prelude::*;

#[task(single)]
pub fn normalize(value: i32) -> Result<i32> {
    Ok(value * 10)
}

register_task! {
    method = normalize,
    name = "normalize",
    author = "Namu",
    version = "0.1.0"
}
