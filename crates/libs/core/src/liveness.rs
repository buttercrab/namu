//! Which values are still live at each point a context grows a node (design §6.2).
//!
//! A new node attaches under the deepest node on its context's chain that still holds a live
//! value; nodes below that hold only dead values and drop out of the chain. So the chain stays as
//! long as the values in use, not as long as the run.

use std::collections::BTreeSet;

use crate::ir::{Next, Operation, Workflow};
use crate::{OpId, ValueId};

/// Live values at the two points of each operation where nodes are created: after its literals
/// and phis are applied, and after its action (a call's outputs, a fan-out's item, a collect's
/// list). Each set is sorted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Liveness {
    after_enter: Vec<Vec<ValueId>>,
    after_action: Vec<Vec<ValueId>>,
}

impl Liveness {
    pub fn after_enter(&self, op: OpId) -> &[ValueId] {
        &self.after_enter[op]
    }

    pub fn after_action(&self, op: OpId) -> &[ValueId] {
        &self.after_action[op]
    }
}

type Set = BTreeSet<ValueId>;

pub fn liveness(workflow: &Workflow) -> Liveness {
    let ops = &workflow.operations;
    let mut live_in: Vec<Set> = vec![Set::new(); ops.len()];
    let mut live_out: Vec<Set> = vec![Set::new(); ops.len()];
    // Backward dataflow to a fixed point.
    let mut changed = true;
    while changed {
        changed = false;
        for id in (0..ops.len()).rev() {
            let op = &ops[id];
            let mut out = Set::new();
            for next in successors(&op.next) {
                let succ = &ops[next];
                let entered = entry_defs(succ);
                out.extend(live_in[next].iter().filter(|v| !entered.contains(v)));
                // A phi reads its source at the end of the edge it arrives by.
                out.extend(
                    succ.phis
                        .iter()
                        .flat_map(|phi| &phi.from)
                        .filter(|(from, _)| *from == id)
                        .map(|(_, value)| *value),
                );
            }
            let input = enter_live(op, &action_live(op, &out));
            if out != live_out[id] || input != live_in[id] {
                live_out[id] = out;
                live_in[id] = input;
                changed = true;
            }
        }
    }
    let after_action: Vec<Set> = ops
        .iter()
        .zip(&live_out)
        .map(|(op, out)| action_live(op, out))
        .collect();
    let after_enter = ops
        .iter()
        .zip(&after_action)
        .map(|(op, after)| before_action(op, after))
        .collect::<Vec<_>>();
    Liveness {
        after_enter: after_enter
            .into_iter()
            .map(|set| set.into_iter().collect())
            .collect(),
        after_action: after_action
            .into_iter()
            .map(|set| set.into_iter().collect())
            .collect(),
    }
}

/// Live after the action: what the successors need, and what `next` reads.
fn action_live(op: &Operation, out: &Set) -> Set {
    let mut live = out.clone();
    match op.next {
        Next::Branch { var, .. } | Next::Return { var: Some(var) } => {
            live.insert(var);
        }
        _ => {}
    }
    live
}

/// Live just before the action (after literals and phis): the action's inputs, and what lives on
/// past it except what it defines.
fn before_action(op: &Operation, after: &Set) -> Set {
    let mut live: Set = after
        .iter()
        .copied()
        .filter(|v| !action_defs(op).contains(v))
        .collect();
    if let Some(call) = &op.call {
        live.extend(call.inputs.iter().copied());
    }
    if let Some(fan_out) = &op.fan_out {
        live.insert(fan_out.input);
    }
    if let Some(collect) = &op.collect {
        live.insert(collect.input);
    }
    live
}

/// Live on entry to the op, before its literals and phis define anything.
fn enter_live(op: &Operation, after: &Set) -> Set {
    let entered = entry_defs(op);
    before_action(op, after)
        .into_iter()
        .filter(|v| !entered.contains(v))
        .collect()
}

fn entry_defs(op: &Operation) -> Set {
    op.literals
        .iter()
        .map(|literal| literal.output)
        .chain(op.phis.iter().map(|phi| phi.output))
        .collect()
}

fn action_defs(op: &Operation) -> Set {
    let call = op.call.iter().flat_map(|call| call.outputs.iter().copied());
    let fan_out = op.fan_out.iter().map(|fan_out| fan_out.output);
    let collect = op.collect.iter().map(|collect| collect.output);
    call.chain(fan_out).chain(collect).collect()
}

fn successors(next: &Next) -> Vec<OpId> {
    match next {
        Next::Jump { next } => vec![*next],
        Next::Branch {
            true_next,
            false_next,
            ..
        } => vec![*true_next, *false_next],
        Next::Return { .. } => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{Call, Literal, Phi};

    fn call(task: &str, inputs: Vec<ValueId>, output: ValueId) -> Option<Call> {
        Some(Call {
            task_id: task.to_string(),
            inputs,
            outputs: vec![output],
        })
    }

    /// `let cfg = 7; let mut x = 0; while cond(x) { x = step(x, cfg) } x`
    fn loop_workflow() -> Workflow {
        let literals = vec![
            Literal {
                output: 0,
                value: "7".into(),
            },
            Literal {
                output: 1,
                value: "0".into(),
            },
        ];
        let header_phi = Phi {
            output: 2,
            from: vec![(0, 1), (2, 4)],
        };
        Workflow::new(
            "loop".into(),
            vec![
                Operation::new(literals, Vec::new(), None, Next::jump(1)),
                // op 1: x = phi; c = cond(x); branch c
                Operation::new(
                    Vec::new(),
                    vec![header_phi],
                    call("cond", vec![2], 3),
                    Next::branch(3, 2, 3),
                ),
                // op 2: x' = step(x, cfg)
                Operation::new(
                    Vec::new(),
                    Vec::new(),
                    call("step", vec![2, 0], 4),
                    Next::jump(1),
                ),
                // op 3: return x
                Operation::new(Vec::new(), Vec::new(), None, Next::return_value(2)),
            ],
        )
    }

    #[test]
    fn a_loop_keeps_only_what_the_next_iteration_needs() {
        let live = liveness(&loop_workflow());
        // After `step`, the old x is dead: only cfg and the new x live on.
        assert_eq!(live.after_action(2), [0, 4]);
        // After the header's phi, x and cfg are live; the previous x' is not.
        assert_eq!(live.after_enter(1), [0, 2]);
        // At the return, only x.
        assert_eq!(live.after_action(3), [2]);
        // After `cond`, its result is read by the branch.
        assert_eq!(live.after_action(1), [0, 2, 3]);
    }
}
