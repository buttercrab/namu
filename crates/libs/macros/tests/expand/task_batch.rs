use namu_macros::task;

#[task(batch)]
fn batch_task(inputs: Vec<i32>) -> Vec<anyhow::Result<i32>>;
