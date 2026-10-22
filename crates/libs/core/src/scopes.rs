//! Static analysis of fan-out scopes.
//!
//! A fan-out opens a scope, and the collect that gathers its branches closes it. Which collect
//! closes which fan-out is fixed by the control-flow graph, so it is found once per workflow
//! instead of being discovered at runtime from whichever branches happen to reach a collect. That
//! is what lets a scope with no items, or whose items were all dropped, still collect `[]`.

use std::collections::HashMap;

use anyhow::{bail, ensure};

use crate::ir::{Next, Workflow};
use crate::{OpId, ValueId};

/// For every operation, the operation that collects the scope it opens. `None` for operations
/// that do not fan out, and for fan-outs whose branches never reach a collect.
pub fn collect_ops(workflow: &Workflow) -> anyhow::Result<Vec<Option<OpId>>> {
    let ops = &workflow.operations;
    ensure!(
        !ops.is_empty(),
        "workflow {} has no operations",
        workflow.name
    );

    // The stack of open scopes (fan-out ops, innermost last) on entry to each op.
    let mut entry: Vec<Option<Vec<OpId>>> = vec![None; ops.len()];
    // The same, after each op's action: inside the scope a fan-out opens, outside the one a
    // collect closes.
    let mut after: Vec<Vec<OpId>> = vec![Vec::new(); ops.len()];
    let mut collect_of = vec![None; ops.len()];
    entry[0] = Some(Vec::new());
    let mut work = vec![0];

    while let Some(op_id) = work.pop() {
        let op = &ops[op_id];
        let mut scopes = entry[op_id].clone().unwrap_or_default();
        if op.fan_out().is_some() {
            scopes.push(op_id);
        }
        if op.collect().is_some() || op.window().is_some() {
            let Some(fan_out) = scopes.pop() else {
                bail!("op {op_id} collects outside of any fan-out");
            };
            match collect_of[fan_out] {
                Some(other) if other != op_id => {
                    bail!(
                        "the fan-out at op {fan_out} is collected by both op {other} and op {op_id}"
                    )
                }
                _ => collect_of[fan_out] = Some(op_id),
            }
        }
        // A window closes the items' scope and opens its windows'.
        if op.window().is_some() {
            scopes.push(op_id);
        }
        after[op_id] = scopes.clone();
        for next in op.successors() {
            ensure!(next < ops.len(), "op {op_id} jumps to missing op {next}");
            match &entry[next] {
                None => {
                    entry[next] = Some(scopes.clone());
                    work.push(next);
                }
                Some(seen) if *seen != scopes => {
                    bail!("op {next} is reached inside different fan-out scopes")
                }
                Some(_) => {}
            }
        }
    }
    check_reduces(workflow, &entry, &after, &collect_of)?;
    Ok(collect_of)
}

/// A reduce's first accumulator is read where its scope was opened, so it must be made outside
/// that scope: before the fan-out, not per item.
fn check_reduces(
    workflow: &Workflow,
    entry: &[Option<Vec<OpId>>],
    after: &[Vec<OpId>],
    collect_of: &[Option<OpId>],
) -> anyhow::Result<()> {
    let ops = &workflow.operations;
    // The scopes each value is made in.
    let mut made_in: HashMap<ValueId, &[OpId]> = HashMap::new();
    for (op_id, op) in ops.iter().enumerate() {
        let Some(entered) = &entry[op_id] else {
            continue;
        };
        let entered_values = op
            .literals
            .iter()
            .map(|literal| literal.output)
            .chain(op.phis.iter().map(|phi| phi.output))
            .chain(op.builtins.iter().map(|builtin| builtin.output));
        for value in entered_values {
            made_in.insert(value, entered);
        }
        for &value in op.action.iter().flat_map(|action| action.outputs()) {
            made_in.insert(value, &after[op_id]);
        }
    }
    for (op_id, op) in ops.iter().enumerate() {
        let Some(reduce) = op.collect().and_then(|collect| collect.reduce.as_ref()) else {
            continue;
        };
        let Some(fan_out) = collect_of.iter().position(|&of| of == Some(op_id)) else {
            continue;
        };
        if made_in
            .get(&reduce.init)
            .is_some_and(|scopes| scopes.contains(&fan_out))
        {
            bail!(
                "the first accumulator of `{}` (value {}) is made inside the fan-out it folds \
                 (op {fan_out}), once per item; make it before the fan-out",
                reduce.task_id,
                reduce.init
            );
        }
    }
    Ok(())
}

/// For every operation that calls a task, the fan-out right after it if that fan-out is the only
/// use of the call's output. A stream task called there can feed the fan-out item by item,
/// without its output ever existing as a whole list. A call with an error edge is not fused: its
/// failure must be caught before anything continues.
pub fn fused_fan_outs(workflow: &Workflow) -> Vec<Option<OpId>> {
    let ops = &workflow.operations;
    let mut uses: HashMap<ValueId, usize> = HashMap::new();
    for op in ops {
        let mut used = |value: ValueId| *uses.entry(value).or_default() += 1;
        op.phis
            .iter()
            .flat_map(|phi| &phi.from)
            .for_each(|(_, value)| used(*value));
        op.builtins
            .iter()
            .flat_map(|builtin| &builtin.inputs)
            .for_each(|&value| used(value));
        if let Some(action) = &op.action {
            action.inputs().for_each(&mut used);
        }
        match op.next {
            Next::Branch { var, .. } | Next::Return { var: Some(var) } => used(var),
            _ => {}
        }
    }
    ops.iter()
        .map(|op| {
            let call = op.call().filter(|call| call.catch.is_none())?;
            let [output] = call.outputs[..] else {
                return None;
            };
            let Next::Jump { next } = op.next else {
                return None;
            };
            let target = ops.get(next)?;
            let fans_out = target.fan_out().is_some_and(|f| f.input == output);
            let only_use = uses.get(&output) == Some(&1);
            (fans_out && target.entered_directly() && only_use).then_some(next)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{Action, Collect, FanOut, Operation};

    fn op(next: Next) -> Operation {
        Operation::new(Vec::new(), Vec::new(), None, next)
    }

    fn fan_out(next: Next) -> Operation {
        let fan_out = FanOut {
            input: 0,
            output: 1,
        };
        Operation::new(Vec::new(), Vec::new(), Some(Action::FanOut(fan_out)), next)
    }

    fn collect(next: Next) -> Operation {
        let collect = Collect {
            input: 1,
            output: 2,
            reduce: None,
        };
        Operation::new(Vec::new(), Vec::new(), Some(Action::Collect(collect)), next)
    }

    fn workflow(operations: Vec<Operation>) -> Workflow {
        Workflow::new("test".to_string(), operations)
    }

    #[test]
    fn a_fan_out_is_paired_with_its_collect() {
        let wf = workflow(vec![
            fan_out(Next::jump(1)),
            op(Next::jump(2)),
            collect(Next::return_unit()),
        ]);
        assert_eq!(collect_ops(&wf).unwrap(), vec![Some(2), None, None]);
    }

    #[test]
    fn nested_scopes_close_innermost_first() {
        let wf = workflow(vec![
            fan_out(Next::jump(1)),
            fan_out(Next::jump(2)),
            collect(Next::jump(3)),
            collect(Next::return_unit()),
        ]);
        assert_eq!(
            collect_ops(&wf).unwrap(),
            vec![Some(3), Some(2), None, None]
        );
    }

    #[test]
    fn a_branch_may_leave_its_scope_by_returning() {
        let wf = workflow(vec![
            fan_out(Next::jump(1)),
            op(Next::branch(0, 2, 3)),
            op(Next::return_unit()),
            collect(Next::return_unit()),
        ]);
        assert_eq!(collect_ops(&wf).unwrap()[0], Some(3));
    }

    #[test]
    fn a_fan_out_without_collect_is_allowed() {
        let wf = workflow(vec![fan_out(Next::jump(1)), op(Next::return_unit())]);
        assert_eq!(collect_ops(&wf).unwrap(), vec![None, None]);
    }

    fn call(output: ValueId, next: Next) -> Operation {
        Operation::new(
            Vec::new(),
            Vec::new(),
            Some(Action::Call(crate::ir::Call {
                task_id: "t".to_string(),
                inputs: Vec::new(),
                outputs: vec![output],
                catch: None,
            })),
            next,
        )
    }

    #[test]
    fn a_call_feeding_only_the_next_fan_out_is_fused() {
        // op 0 calls into value 0, op 1 fans out over it.
        let wf = workflow(vec![
            call(0, Next::jump(1)),
            fan_out(Next::jump(2)),
            collect(Next::return_unit()),
        ]);
        assert_eq!(fused_fan_outs(&wf), vec![Some(1), None, None]);
    }

    #[test]
    fn a_call_whose_output_is_used_again_is_not_fused() {
        let wf = workflow(vec![
            call(0, Next::jump(1)),
            fan_out(Next::jump(2)),
            collect(Next::return_value(0)),
        ]);
        assert_eq!(fused_fan_outs(&wf), vec![None, None, None]);
    }

    #[test]
    fn collecting_outside_a_scope_is_rejected() {
        let wf = workflow(vec![collect(Next::return_unit())]);
        assert!(collect_ops(&wf).is_err());
    }

    #[test]
    fn merging_paths_must_agree_on_scopes() {
        // op 3 is reached both inside the scope (via op 1) and after its collect (via op 2).
        let wf = workflow(vec![
            fan_out(Next::jump(1)),
            op(Next::branch(0, 2, 3)),
            collect(Next::jump(3)),
            op(Next::return_unit()),
        ]);
        assert!(collect_ops(&wf).is_err());
    }
}
