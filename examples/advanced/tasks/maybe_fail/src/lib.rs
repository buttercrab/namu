use namu::prelude::*;

#[task(single)]
pub fn maybe_fail(value: i32) -> Result<i32> {
    if value == 4 {
        anyhow::bail!("intentional failure for demo");
    }
    Ok(value)
}

register_task! {
    method = maybe_fail,
    name = "maybe_fail",
    author = "Namu",
    version = "0.1.0"
}
