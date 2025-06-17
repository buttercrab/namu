use namu_macros::{task, workflow};

#[task]
fn less_than(a: i32, b: i32) -> anyhow::Result<bool> {
    Ok(a < b)
}

#[task]
fn add_one(a: i32) -> anyhow::Result<i32> {
    Ok(a + 1)
}

#[workflow]
fn while_loop_workflow() -> i32 {
    let mut i = 0;
    while less_than(i, 5) {
        i = add_one(i);
    }
    i
}
