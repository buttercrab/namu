use macros::{task, workflow};

#[task]
fn less_than(a: i32, b: i32) -> anyhow::Result<bool> {
    Ok(a < b)
}

#[task]
fn is_even(a: i32) -> anyhow::Result<bool> {
    Ok(a % 2 == 0)
}

#[task]
fn add_one(a: i32) -> anyhow::Result<i32> {
    Ok(a + 1)
}

#[task]
fn add_two(a: i32) -> anyhow::Result<i32> {
    Ok(a + 2)
}

#[workflow]
fn nested_if_in_while_workflow() -> i32 {
    let mut i = 0;
    while less_than(i, 10) {
        if is_even(i) {
            i = add_two(i);
        } else {
            i = add_one(i);
        }
    }
    i
}
