use namu_macros::{task, workflow};

#[task]
fn add_one(a: i32) -> anyhow::Result<i32>;

#[task]
fn multiply_by_two(a: i32) -> anyhow::Result<i32>;

#[workflow]
fn chained_tasks_workflow() -> i32 {
    let initial = 5;
    let added = add_one(initial);
    multiply_by_two(added)
}
