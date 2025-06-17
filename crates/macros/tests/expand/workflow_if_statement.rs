use namu_macros::{task, workflow};

#[task]
fn do_nothing(a: i32) -> anyhow::Result<i32> {
    Ok(a)
}

#[workflow]
fn if_statement_workflow() {
    let x = 10;
    if x > 5 {
        do_nothing(x);
    }
}
