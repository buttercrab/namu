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
        split(a, 3)
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

#[test]
fn engine_stream_failure_only_stops_leaf() {
    #[workflow]
    fn stream_failure_workflow() -> i32 {
        let a = range(1, 4);
        maybe_fail(a)
    }

    let graph = stream_failure_workflow();
    let wf_ir = graph.to_serializable("stream_failure".to_string());

    let result_val = run_workflow(wf_ir);

    let vals = result_val
        .iter()
        .map(|v| *v.downcast_ref::<i32>().unwrap())
        .sorted()
        .collect::<Vec<_>>();
    assert_eq!(vals, vec![10, 30]);
}

#[test]
fn stream_task_setup_error_does_not_panic() {
    #[workflow]
    fn stream_error_workflow() -> i32 {
        let a = 1;
        fail_stream(a)
    }

    let graph = stream_error_workflow();
    let wf_ir = graph.to_serializable("stream_error".to_string());
    let result_val = run_workflow(wf_ir);

    assert!(result_val.is_empty());
}

#[test]
fn loop_phis_are_parallel_assignments() {
    // `t = a; a = b; b = t;` swaps a and b. Phis that were applied one at a time used to read
    // the freshly written `a` and produce a == b == 2.
    #[workflow]
    fn swap_workflow() -> i32 {
        let mut a = 1;
        let mut b = 2;
        let mut i = 0;
        while less_than(i, 1) {
            let t = a;
            a = b;
            b = t;
            i = add(i, 1);
        }
        add(add(a, a), b)
    }

    let graph = swap_workflow();
    let result_val = run_workflow(graph.to_serializable("swap".to_string()));
    // a = 2, b = 1 -> 2 + 2 + 1
    assert_eq!(*result_val[0].downcast_ref::<i32>().unwrap(), 5);
}

#[test]
fn string_literals_keep_escaped_characters() {
    #[workflow]
    fn exclaim_workflow() -> String {
        let s = "say \"hi\"\n";
        exclaim(s)
    }

    let result_val = run_workflow(exclaim_workflow().to_serializable("exclaim".to_string()));
    assert_eq!(
        result_val[0].downcast_ref::<String>().unwrap(),
        "say \"hi\"\n!"
    );
}
