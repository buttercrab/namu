use macros::task;

#[task]
fn no_args_task() -> anyhow::Result<()> {
    Ok(())
}
