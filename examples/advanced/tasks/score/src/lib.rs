use namu::prelude::*;

#[task(single)]
pub fn score(value: i32) -> Result<i32> {
    Ok(value + 7)
}

register_task! {
    method = score,
    name = "score",
    author = "Namu",
    version = "0.1.0"
}
