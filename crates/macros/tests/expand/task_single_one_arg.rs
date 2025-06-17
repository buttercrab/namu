use namu_macros::task;

#[task]
fn single_arg_task(a: i32) -> anyhow::Result<i32> {
    Ok(a)
}
