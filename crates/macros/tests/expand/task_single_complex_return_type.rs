use namu_macros::task;

pub struct MyComplexType {
    pub value: String,
}

#[task]
fn complex_return_task(a: i32) -> anyhow::Result<MyComplexType> {
    Ok(MyComplexType {
        value: a.to_string(),
    })
}
