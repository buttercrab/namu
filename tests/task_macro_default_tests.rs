mod common;

use namu::prelude::*;

use crate::common::run_workflow;

#[task]
pub fn add_one(x: i32) -> Result<i32> {
    Ok(x + 1)
}

register_task! { method = add_one, name = "add_one", author = "test", version = "0.1" }

#[test]
fn task_macro_defaults_to_single_for_functions() {
    #[workflow]
    fn wf() -> i32 {
        let x = 41;
        add_one(x)
    }

    let graph = wf();
    let wf_ir = graph.to_serializable("default_task".to_string());
    let result = run_workflow(wf_ir);

    let val = *result[0].downcast_ref::<i32>().unwrap();
    assert_eq!(val, 42);
}
