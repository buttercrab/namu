use macros::workflow;

#[workflow]
fn simple_return_workflow() -> i32 {
    123
}
