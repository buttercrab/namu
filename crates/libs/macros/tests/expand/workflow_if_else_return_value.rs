use namu_macros::{task, workflow};

#[task]
fn double(a: i32) -> anyhow::Result<i32>;

#[task]
fn identity(a: i32) -> anyhow::Result<i32>;

#[workflow]
fn if_else_return_value_workflow() -> i32 {
    let x = 10;
    if x > 5 { double(x) } else { identity(x) }
}
