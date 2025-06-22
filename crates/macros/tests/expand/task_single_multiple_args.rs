use namu_macros::task;

#[task(single)]
fn multiple_args_task(a: i32, b: String) -> anyhow::Result<String> {
    Ok(format!("{}{}", a, b))
}
