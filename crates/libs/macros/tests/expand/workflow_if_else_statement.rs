use namu_macros::{task, workflow};

#[task]
fn task_a(a: i32) -> anyhow::Result<i32>;

#[task]
fn task_b(a: i32) -> anyhow::Result<i32>;

#[workflow]
fn if_else_statement_workflow() {
    let x = 10;
    if x > 20 {
        task_a(x);
    } else {
        task_b(x);
    }
}
