use namu_macros::{task, workflow};

#[task(single)]
fn less_than(a: i32, b: i32) -> anyhow::Result<bool> {
    Ok(a < b)
}

#[task(single)]
fn add(a: i32, b: i32) -> anyhow::Result<i32> {
    Ok(a + b)
}

#[workflow]
fn multiple_mutable_vars_workflow() -> i32 {
    let mut a = 0;
    let mut b = 1;
    let mut i = 0;

    while less_than(i, 5) {
        let temp = a;
        a = b;
        b = add(temp, b);
        i = add(i, 1);
    }

    a
}
