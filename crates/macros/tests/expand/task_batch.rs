use namu_macros::task;

#[task(batch)]
fn batch_task(inputs: Vec<i32>) -> Vec<anyhow::Result<i32>> {
    inputs.into_iter().map(|i| Ok(i * 2)).collect()
}
