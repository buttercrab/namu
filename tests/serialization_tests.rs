#![allow(dead_code)]

mod common;

use crate::common::*;
use namu::workflow;
use namu_core::ir::Next;

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

    // Expect exactly 4 grouped operations in SSA form
    assert_eq!(ops.len(), 4);

    // Op0: Literal(10) + is_positive call -> Branch
    assert_eq!(ops[0].literals.len(), 1);
    assert_eq!(ops[0].literals[0].output, 0);
    assert_eq!(ops[0].literals[0].value, "10");
    assert!(matches!(ops[0].next, Next::Branch { .. }));

    // Find operations with double and identity calls irrespective of exact indices.
    let mut double_op_idx = None;
    let mut identity_op_idx = None;
    for (idx, op) in ops.iter().enumerate() {
        if let Some(call) = &op.call {
            if call.task_id.contains("double") {
                double_op_idx = Some(idx);
                assert_eq!(call.inputs, vec![0]);
                assert_eq!(call.outputs, vec![2]);
            } else if call.task_id.contains("identity") {
                identity_op_idx = Some(idx);
                assert_eq!(call.inputs, vec![0]);
                assert_eq!(call.outputs, vec![3]);
            }
        }
    }
    let double_idx = double_op_idx.expect("double call op not found");
    let identity_idx = identity_op_idx.expect("identity call op not found");

    // Both should jump to the merge operation (last index)
    let merge_idx = ops.len() - 1;
    assert!(matches!(ops[double_idx].next, Next::Jump { next } if next == merge_idx));
    assert!(matches!(ops[identity_idx].next, Next::Jump { next } if next == merge_idx));

    // Op3: Phi merge -> Return
    {
        assert!(ops[3].phis.len() == 1);
        let phi = &ops[3].phis[0];
        assert_eq!(phi.from, vec![(1, 2), (2, 3)]);
        assert_eq!(phi.output, 4);
    }
    assert!(matches!(ops[3].next, Next::Return { var: Some(4) }));

    println!("OPS LEN: {}", ops.len());
    for (idx, op) in ops.iter().enumerate() {
        println!(
            "op {idx}: literals={} phis={} call={} next={:?}",
            op.literals.len(),
            op.phis.len(),
            op.call.is_some(),
            op.next
        );
        if let Some(call) = &op.call {
            println!(
                "    call task: {} inputs={:?} outputs={:?}",
                call.task_id, call.inputs, call.outputs
            );
        }
        for lit in &op.literals {
            println!("    lit {} = {}", lit.output, lit.value);
        }
        for phi in &op.phis {
            println!("    phi {} from {:?}", phi.output, phi.from);
        }
    }
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

    // Expect 4 or 5 operations depending on whether an empty exit placeholder is created.
    assert!(ops.len() >= 4);

    // Op0 Literal -> Jump 1
    assert!(!ops[0].literals.is_empty());
    assert!(matches!(ops[0].next, Next::Jump { next: 1 }));

    // Find branch op (should be 1)
    assert!(matches!(ops[1].next, Next::Branch { .. }));

    // Op2 body Jump back
    assert!(matches!(ops[2].next, Next::Jump { .. }));

    // Op3 Return
    assert!(matches!(ops.last().unwrap().next, Next::Return { .. }));
}
