use namu::prelude::*;

#[task(batch, batch_size = 4)]
pub fn embed_batch(inputs: Vec<i32>) -> Vec<Result<i32>> {
    inputs.into_iter().map(|value| Ok(value * 2)).collect()
}

register_task! {
    method = embed_batch,
    name = "embed_batch",
    author = "Namu",
    version = "0.1.0"
}
