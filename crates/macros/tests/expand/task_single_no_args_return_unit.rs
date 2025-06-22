use namu_macros::task;

#[task(single)]
fn no_args_task() -> anyhow::Result<()> {
    Ok(())
}
