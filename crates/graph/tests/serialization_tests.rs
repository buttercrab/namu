#![allow(dead_code)]

mod common;

use crate::common::*;
use ::common::{Next, Task};
use graph::workflow;

#[test]
fn serializable_conditional_workflow() {
    #[workflow]
    fn test_workflow() -> i32 {
        let input = 10;
        if is_positive(input) {
            double(input)
        } else {
            identity(input)
        }
    }

    let graph = test_workflow();
    let serializable = graph.to_serializable("conditional".to_string());
    let ops = serializable.operations;

    // Expected: 4 operations
    // 1. Literal(10), Task(is_positive) -> Branch
    // 2. Phi -> Return
    // 3. Task(double) -> Jump
    // 4. Task(identity) -> Jump
    assert_eq!(ops.len(), 4);

    // Op 0: is_positive and branch
    assert!(ops[0].phis.is_empty());
    assert_eq!(ops[0].literals.len(), 1);
    assert!(matches!(ops[0].task, Some(Task { ref name, .. }) if name.contains("is_positive")));
    assert!(matches!(
        ops[0].next,
        Next::Branch {
            var: 1,
            true_next: 2,
            false_next: 3
        }
    ));

    // Op 1: Merge block
    assert_eq!(ops[1].phis.len(), 1);
    assert!(ops[1].literals.is_empty());
    assert!(ops[1].task.is_none());
    assert!(matches!(ops[1].next, Next::Return { .. }));

    // Op 2: double
    assert!(ops[2].phis.is_empty());
    assert!(ops[2].literals.is_empty());
    assert!(matches!(ops[2].task, Some(Task { ref name, .. }) if name.contains("double")));
    assert!(matches!(ops[2].next, Next::Jump { next: 1 }));

    // Op 3: identity
    assert!(ops[3].phis.is_empty());
    assert!(ops[3].literals.is_empty());
    assert!(matches!(ops[3].task, Some(Task { ref name, .. }) if name.contains("identity")));
    assert!(matches!(ops[3].next, Next::Jump { next: 1 }));
}

#[test]
fn serializable_while_loop_workflow() {
    #[workflow]
    fn test_workflow() -> i32 {
        let mut i = 0;
        while less_than(i, 3) {
            i = add(i, 1);
        }
        i
    }

    let graph = test_workflow();
    let serializable = graph.to_serializable("while_loop".to_string());
    let ops = serializable.operations;

    // Expected: 4 operations
    // 1. Entry: Literal(0) -> Jump
    // 2. Header: Phi, Literal(3), Task(less_than) -> Branch
    // 3. Body: Literal(1), Task(add) -> Jump
    // 4. Exit: Synthetic empty operation -> Return
    assert_eq!(ops.len(), 4);

    // Op 1: Header
    let header_op = &ops[1];
    assert_eq!(header_op.phis.len(), 1);
    assert_eq!(header_op.literals.len(), 1);
    assert!(header_op.task.is_some());

    assert!(matches!(
        header_op.next,
        Next::Branch {
            true_next: 2,
            false_next: 3,
            ..
        }
    ));

    // Op 3: Synthetic Return
    let exit_op = &ops[3];
    assert!(exit_op.phis.is_empty());
    assert!(exit_op.literals.is_empty());
    assert!(exit_op.task.is_none());
    assert!(matches!(exit_op.next, Next::Return { .. }));
}

#[test]
fn serializes_to_json() {
    #[workflow]
    fn test_workflow() -> i32 {
        let input = 10;
        if is_positive(input) {
            double(input)
        } else {
            identity(input)
        }
    }

    let graph = test_workflow();
    let serializable = graph.to_serializable("json_test".to_string());

    let json = serde_json::to_string(&serializable).unwrap();
    assert!(!json.is_empty());

    // A simple check to ensure the output looks like a valid JSON
    assert!(json.starts_with("{\"name\":\"json_test\",\"operations\":["));
}
