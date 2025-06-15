use macros::task;

#[task(type = "stream")]
fn stream_task(input: i32) -> impl Iterator<Item = anyhow::Result<i32>> {
    (0..input).map(Ok)
}
