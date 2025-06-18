#![allow(dead_code)]

mod common;

use crate::common::*;
use namu::workflow;
use namu_core::ir::{Next, OpKind};

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

    // Expect exactly 5 operations in SSA form
    assert_eq!(ops.len(), 5);

    // Op0: Literal(10) -> Jump
    assert!(matches!(ops[0].kind, OpKind::Literal { .. }));
    assert_eq!(ops[0].outputs, vec![0]);
    assert!(matches!(ops[0].next, Next::Jump { next: 1 }));

    // Op1: Call(is_positive) -> Branch
    match &ops[1].kind {
        OpKind::Call { name, inputs } => {
            assert!(name.contains("is_positive"));
            assert_eq!(inputs, &vec![0]);
        }
        _ => panic!("Op1 not Call"),
    }
    assert_eq!(ops[1].outputs, vec![1]);
    assert!(matches!(
        ops[1].next,
        Next::Branch {
            var: 1,
            true_next: 2,
            false_next: 3
        }
    ));

    // Op2: Call(double) -> Jump 4
    match &ops[2].kind {
        OpKind::Call { name, inputs } => {
            assert!(name.contains("double"));
            assert_eq!(inputs, &vec![0]);
        }
        _ => panic!("Op2 not Call double"),
    }
    assert_eq!(ops[2].outputs, vec![2]);
    assert!(matches!(ops[2].next, Next::Jump { next: 4 }));

    // Op3: Call(identity) -> Jump 4
    match &ops[3].kind {
        OpKind::Call { name, inputs } => {
            assert!(name.contains("identity"));
            assert_eq!(inputs, &vec![0]);
        }
        _ => panic!("Op3 not Call identity"),
    }
    assert_eq!(ops[3].outputs, vec![3]);
    assert!(matches!(ops[3].next, Next::Jump { next: 4 }));

    // Op4: Phi merge -> Return
    match &ops[4].kind {
        OpKind::Phi { from } => {
            assert_eq!(from, &vec![(2, 2), (3, 3)]);
        }
        _ => panic!("Op4 not Phi"),
    }
    assert_eq!(ops[4].outputs, vec![4]);
    assert!(matches!(ops[4].next, Next::Return { var: Some(4) }));
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

    // Expect 5 operations (literal, header phi+call, body call, exit placeholder, merge return)
    assert_eq!(ops.len(), 5);

    // Op0 Literal -> Jump 1
    assert!(matches!(ops[0].kind, OpKind::Literal { .. }));
    assert!(matches!(ops[0].next, Next::Jump { next: 1 }));

    // Find branch op (should be 1)
    assert!(matches!(ops[1].next, Next::Branch { .. }));

    // Op2 body Jump back
    assert!(matches!(ops[2].next, Next::Jump { next: 1 }));

    // Op4 Return
    assert!(matches!(ops[4].next, Next::Return { .. }));
}
