//! Calls in one branch that can run at the same time (design §5: independent calls in a block run
//! in parallel).
//!
//! A group starts at a call and takes the calls that follow it in a straight line: each reached
//! by a plain jump from the one before, with no literals or phis of its own, reading none of the
//! group's earlier outputs. None of them may feed a stream fusion (its fan-out must start right
//! after it). Every call in a group reads only values that exist before the group, so they can all
//! start at once; their outputs are stored in order afterwards.

use std::collections::HashSet;

use crate::OpId;
use crate::ir::{Next, Workflow};

/// For every op that starts a group of two or more calls, the ops of the group, in order.
pub fn parallel_calls(workflow: &Workflow, fused: &[Option<OpId>]) -> Vec<Option<Vec<OpId>>> {
    let ops = &workflow.operations;
    let groupable = |op: OpId| ops[op].call().is_some() && fused[op].is_none();
    (0..ops.len())
        .map(|first| {
            if !groupable(first) {
                return None;
            }
            let mut group = vec![first];
            let mut outputs: HashSet<_> =
                ops[first].action.iter().flat_map(|a| a.outputs()).collect();
            while let Next::Jump { next } = ops[*group.last().expect("not empty")].next {
                let op = &ops[next];
                let independent = op
                    .call()
                    .is_some_and(|call| call.inputs.iter().all(|input| !outputs.contains(input)));
                if !groupable(next)
                    || !op.literals.is_empty()
                    || !op.phis.is_empty()
                    || !independent
                {
                    break;
                }
                outputs.extend(op.action.iter().flat_map(|a| a.outputs()));
                group.push(next);
            }
            (group.len() > 1).then_some(group)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ValueId;
    use crate::ir::{Action, Call, Operation};

    fn call(inputs: Vec<ValueId>, output: ValueId, next: Next) -> Operation {
        let call = Call {
            task_id: "t".into(),
            inputs,
            outputs: vec![output],
        };
        Operation::new(Vec::new(), Vec::new(), Some(Action::Call(call)), next)
    }

    #[test]
    fn independent_calls_group_until_one_reads_an_earlier_output() {
        // a = f(x); b = g(x); c = h(a); return c
        let workflow = Workflow::new(
            "w".into(),
            vec![
                call(vec![0], 1, Next::jump(1)),
                call(vec![0], 2, Next::jump(2)),
                call(vec![1], 3, Next::return_value(3)),
            ],
        );
        let groups = parallel_calls(&workflow, &[None, None, None]);
        // From op 0, h reads a, made in the group: it waits. A branch that resumes at op 1 (say,
        // on another worker) already has a, so g and h can run together.
        assert_eq!(groups, vec![Some(vec![0, 1]), Some(vec![1, 2]), None]);
    }

    #[test]
    fn a_fused_stream_call_is_never_grouped() {
        let workflow = Workflow::new(
            "w".into(),
            vec![
                call(vec![0], 1, Next::jump(1)),
                call(vec![0], 2, Next::return_value(2)),
            ],
        );
        assert_eq!(
            parallel_calls(&workflow, &[None, Some(9)]),
            vec![None, None]
        );
    }
}
