use namu_macros::{task, workflow};

#[task(single)]
fn task_a(a: i32) -> anyhow::Result<i32> {
    Ok(a)
}

#[task(single)]
fn task_b(a: i32) -> anyhow::Result<i32> {
    Ok(a)
}

#[workflow]
fn if_else_statement_workflow() {
    let x = 10;
    if x > 20 {
        task_a(x);
    } else {
        task_b(x);
    }
}
