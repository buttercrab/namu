use namu_macros::{task, workflow};

#[task(single)]
fn double(a: i32) -> anyhow::Result<i32> {
    Ok(a * 2)
}

#[task(single)]
fn identity(a: i32) -> anyhow::Result<i32> {
    Ok(a)
}

#[workflow]
fn if_else_return_value_workflow() -> i32 {
    let x = 10;
    if x > 5 { double(x) } else { identity(x) }
}
