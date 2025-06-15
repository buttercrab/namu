use macros::task;

#[task(stream)]
fn stream_task(input: i32) -> anyhow::Result<impl Iterator<Item = anyhow::Result<i32>>> {
    Ok((0..input).map(Ok))
}
