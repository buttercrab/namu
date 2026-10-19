//! Calls in one branch that can overlap (design §5: a block is a dataflow DAG, and independent
//! calls in it run in parallel).
//!
//! A group starts at a call and takes the calls that follow it in a straight line: each reached by
//! a plain jump from the one before, with no literals or phis of its own. None of them may feed a
//! stream fusion (its fan-out must start right after it) or have an error edge (where control goes
//! depends on how it ends). Within the group the calls form a DAG by the outputs they read: each
//! starts once the calls it reads from have finished, so a call that reads none of them starts at
//! once. A straight run in which every call reads the one before is a chain, with nothing to
//! overlap, and is not a group.

use std::collections::HashMap;

use crate::OpId;
use crate::ir::{Next, Workflow};

/// For every op that starts a group of calls that can overlap, the ops of the group, in order.
pub fn parallel_calls(workflow: &Workflow, fused: &[Option<OpId>]) -> Vec<Option<Vec<OpId>>> {
    let ops = &workflow.operations;
    let groupable =
        |op: OpId| ops[op].call().is_some_and(|call| call.catch.is_none()) && fused[op].is_none();
    let entered_directly = |op: OpId| ops[op].literals.is_empty() && ops[op].phis.is_empty();
    (0..ops.len())
        .map(|first| {
            if !groupable(first) {
                return None;
            }
            let mut group = vec![first];
            while let Next::Jump { next } = ops[*group.last().expect("not empty")].next {
                if !groupable(next) || !entered_directly(next) || group.contains(&next) {
                    break;
                }
                group.push(next);
            }
            (depth(workflow, &group) < group.len()).then_some(group)
        })
        .collect()
}

/// The length of the longest chain of calls in `group` that each read an output of the one
/// before: how many rounds the group takes when every call starts as soon as it can.
fn depth(workflow: &Workflow, group: &[OpId]) -> usize {
    let mut made_at: HashMap<_, usize> = HashMap::new();
    let mut deepest = 0;
    for &op in group {
        let call = workflow.operations[op].call().expect("a group holds calls");
        let depth = 1 + call
            .inputs
            .iter()
            .filter_map(|input| made_at.get(input))
            .max()
            .unwrap_or(&0);
        made_at.extend(call.outputs.iter().map(|&output| (output, depth)));
        deepest = deepest.max(depth);
    }
    deepest
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
            catch: None,
        };
        Operation::new(Vec::new(), Vec::new(), Some(Action::Call(call)), next)
    }

    #[test]
    fn a_straight_run_of_calls_is_a_group_unless_it_is_a_chain() {
        // a = f(x); b = g(a); c = h(x); return c: h overlaps f and g.
        let workflow = Workflow::new(
            "w".into(),
            vec![
                call(vec![0], 1, Next::jump(1)),
                call(vec![1], 2, Next::jump(2)),
                call(vec![0], 3, Next::return_value(3)),
            ],
        );
        let groups = parallel_calls(&workflow, &[None, None, None]);
        assert_eq!(groups, vec![Some(vec![0, 1, 2]), Some(vec![1, 2]), None]);

        // a = f(x); b = g(a); return b: a chain.
        let chain = Workflow::new(
            "w".into(),
            vec![
                call(vec![0], 1, Next::jump(1)),
                call(vec![1], 2, Next::return_value(2)),
            ],
        );
        assert_eq!(parallel_calls(&chain, &[None, None]), vec![None, None]);
    }

    #[test]
    fn a_loop_of_calls_is_not_followed_around() {
        let workflow = Workflow::new(
            "w".into(),
            vec![
                call(vec![0], 1, Next::jump(1)),
                call(vec![0], 2, Next::jump(0)),
            ],
        );
        assert_eq!(
            parallel_calls(&workflow, &[None, None]),
            vec![Some(vec![0, 1]), Some(vec![1, 0])]
        );
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
