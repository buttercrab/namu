use namu::prelude::*;

#[task(stream)]
pub fn id_range(start: i32, end: i32) -> Result<impl Iterator<Item = Result<i32>>> {
    Ok((start..end).map(Ok))
}

register_task! {
    method = id_range,
    name = "id_range",
    author = "Namu",
    version = "0.1.0"
}
