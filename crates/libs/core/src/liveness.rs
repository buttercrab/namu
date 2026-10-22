//! Which values are still live at each point a context grows a node (design §6.2).
//!
//! A new node attaches under the deepest node on its context's chain that still holds a live
//! value; nodes below that hold only dead values and drop out of the chain. So the chain stays as
//! long as the values in use, not as long as the run.

use std::collections::BTreeSet;

use crate::ir::{Next, Operation, Workflow};
use crate::{OpId, ValueId};

/// Live values at the two points of each operation where nodes are created: after its literals,
/// phis and built-ins are applied, and after its action (a call's outputs, a fan-out's item, a
/// collect's list). Each set is sorted.
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
            for next in op.successors() {
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
    if let Some(action) = &op.action {
        live.extend(action.inputs());
    }
    live
}

/// Live on entry to the op, before its literals, phis and built-ins define anything. Built-ins run
/// in order, so each reads what is live after the ones before it.
fn enter_live(op: &Operation, after: &Set) -> Set {
    let mut live = before_action(op, after);
    for builtin in op.builtins.iter().rev() {
        live.remove(&builtin.output);
        live.extend(builtin.inputs.iter().copied());
    }
    let entered = entry_defs(op);
    live.into_iter().filter(|v| !entered.contains(v)).collect()
}

fn entry_defs(op: &Operation) -> Set {
    op.literals
        .iter()
        .map(|literal| literal.output)
        .chain(op.phis.iter().map(|phi| phi.output))
        .chain(op.builtins.iter().map(|builtin| builtin.output))
        .collect()
}

/// What the action defines: its outputs, and a call's error on its error edge.
fn action_defs(op: &Operation) -> Set {
    op.action
        .iter()
        .flat_map(|action| action.outputs().iter().copied())
        .chain(op.catch().map(|catch| catch.error))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{Action, Call, Catch, Literal, Phi};

    fn call(task: &str, inputs: Vec<ValueId>, output: ValueId) -> Option<Action> {
        Some(Action::Call(Call {
            task_id: task.to_string(),
            inputs,
            outputs: vec![output],
            catch: None,
        }))
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

    #[test]
    fn an_error_edge_keeps_what_the_handler_reads() {
        // `match attempt(t(a)) { Ok(v) => v, Err(e) => u(a, e) }`, with `a` = 0.
        let mut attempted = call("t", vec![0], 1);
        if let Some(Action::Call(call)) = &mut attempted {
            call.catch = Some(Catch { error: 2, next: 2 });
        }
        let workflow = Workflow::new(
            "attempt".into(),
            vec![
                Operation::new(Vec::new(), Vec::new(), attempted, Next::jump(1)),
                Operation::new(Vec::new(), Vec::new(), None, Next::return_value(1)),
                Operation::new(
                    Vec::new(),
                    Vec::new(),
                    call("u", vec![0, 2], 3),
                    Next::return_value(3),
                ),
            ],
        );
        let live = liveness(&workflow);
        // The handler reads `a`, so it outlives the call; the error is defined by the call.
        assert_eq!(live.after_enter(0), [0]);
        assert_eq!(live.after_action(0), [0, 1, 2]);
        assert_eq!(live.after_enter(2), [0, 2]);
    }

    #[test]
    fn a_built_in_reads_what_the_ones_before_it_made() {
        // op 0: a = x + 1; b = a * 2; then call t(b, y), returning its output.
        use crate::ir::{Builtin, BuiltinOp, ScalarType};
        let builtin = |op, inputs: Vec<ValueId>, output| Builtin {
            op,
            ty: ScalarType::I64,
            inputs,
            output,
        };
        let workflow = Workflow::new(
            "builtins".into(),
            vec![
                Operation::new(
                    Vec::new(),
                    Vec::new(),
                    call("t", vec![11, 1], 12),
                    Next::return_value(12),
                )
                .with_builtins(vec![
                    builtin(BuiltinOp::Add, vec![0, 0], 10),
                    builtin(BuiltinOp::Mul, vec![10, 10], 11),
                ]),
            ],
        );
        let live = liveness(&workflow);
        // After the step, `a` is dead: only `b` and `y` reach the call.
        assert_eq!(live.after_enter(0), [1, 11]);
        assert_eq!(live.after_action(0), [12]);
    }
}
