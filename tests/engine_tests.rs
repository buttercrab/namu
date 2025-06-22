// new file

mod common;

use itertools::Itertools;
use namu::workflow;

use crate::common::*;

// ---- The actual test ------------------------------------------------------

#[test]
fn engine_executes_simple_workflow() {
    // 1. Build workflow IR.
    #[workflow]
    fn simple_workflow() -> i32 {
        let a = 1;
        let b = 2;
        add(a, b)
    }

    let graph = simple_workflow();
    let wf_ir = graph.to_serializable("simple".to_string());

    // 2. Execute workflow via helper.
    let result_val = run_workflow(wf_ir);

    // 3. Assert result.
    let val = *result_val[0].downcast_ref::<i32>().unwrap();
    assert_eq!(val, 3);
}

// ---- Fibonacci workflow test --------------------------------------------

#[test]
fn engine_executes_fibonacci_workflow() {
    // 1. Build workflow IR.
    #[workflow]
    fn fibonacci_workflow() -> i32 {
        let mut a = 0;
        let mut b = 1;

        while less_than(a, 10) {
            let c = add(a, b);
            a = b;
            b = c;
        }

        b
    }

    let graph = fibonacci_workflow();
    let wf_ir = graph.to_serializable("fibonacci".to_string());

    // 2. Execute workflow via helper.
    let result_val = run_workflow(wf_ir);

    // 3. Assert result.
    let val = *result_val[0].downcast_ref::<i32>().unwrap();
    assert_eq!(val, 21);
}

#[test]
fn engine_executes_list_workflow() {
    #[workflow]
    fn list_workflow() -> i32 {
        let a = range(1, 4);
        let b = split(a, 3);
        b
    }

    let graph = list_workflow();
    let wf_ir = graph.to_serializable("list".to_string());

    let result_val = run_workflow(wf_ir);

    let vals = result_val
        .iter()
        .map(|v| *v.downcast_ref::<i32>().unwrap())
        .sorted()
        .collect::<Vec<_>>();
    assert_eq!(vals, vec![10, 11, 12, 20, 21, 22, 30, 31, 32]);
}
