use macros::{task, workflow};

#[task]
fn is_positive(a: i32) -> anyhow::Result<bool> {
    Ok(a > 0)
}

#[task]
fn action_if_true() -> anyhow::Result<()> {
    Ok(())
}

#[workflow]
fn if_with_task_in_condition_workflow() {
    let x = 10;
    if is_positive(x) {
        action_if_true();
    }
}
