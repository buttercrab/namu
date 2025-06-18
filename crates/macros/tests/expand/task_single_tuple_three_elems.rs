use namu_macros::task;

#[task]
fn triple(a: i32) -> anyhow::Result<(i32, bool, String)> {
    Ok((a, a > 0, a.to_string()))
}
