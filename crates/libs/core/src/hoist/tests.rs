use super::*;
use crate::ir::{Collect, FanOut};

const ITEMS: ValueId = 0;
const ITEM: ValueId = 1;
const PATH: ValueId = 2;
const MODEL: ValueId = 3;
const LABEL: ValueId = 4;
const LABELS: ValueId = 5;

fn op(action: Action, next: Next) -> Operation {
    Operation::new(Vec::new(), Vec::new(), Some(action), next)
}

fn call(task: &str, inputs: &[ValueId], output: ValueId, next: Next) -> Operation {
    op(
        Action::Call(Call {
            task_id: task.to_string(),
            inputs: inputs.to_vec(),
            outputs: vec![output],
            catch: None,
        }),
        next,
    )
}

fn fan_out(next: Next) -> Operation {
    op(
        Action::FanOut(FanOut {
            input: ITEMS,
            output: ITEM,
        }),
        next,
    )
}

fn collect() -> Operation {
    op(
        Action::Collect(Collect {
            input: LABEL,
            output: LABELS,
            reduce: None,
        }),
        Next::return_value(LABELS),
    )
}

fn workflow(operations: Vec<Operation>) -> Workflow {
    Workflow::new("wf".to_string(), operations)
}

fn tasks(workflow: &Workflow) -> Vec<Option<&str>> {
    let ops = workflow.operations.iter();
    ops.map(|op| op.call().map(|call| &call.task_id[..]))
        .collect()
}

/// `for item in items { classify(load(path), item) }`, with `load` at op 1.
fn loads_per_item(load: Next) -> Vec<Operation> {
    vec![
        fan_out(Next::jump(1)),
        call("load", &[PATH], MODEL, load),
        call("classify", &[MODEL, ITEM], LABEL, Next::jump(3)),
        collect(),
    ]
}

#[test]
fn a_call_every_branch_makes_alike_is_made_before_the_fan_out() {
    let wf = workflow(loads_per_item(Next::jump(2)));
    let shared = hoist(&wf, |_| true).unwrap();
    assert_eq!(tasks(&shared), [Some("load"), None, Some("classify"), None]);
    assert!(shared.operations[1].fan_out().is_some());
    assert_eq!(shared.operations[0].next, Next::jump(1));
    assert_eq!(shared.operations[1].next, Next::jump(2));
    // Nothing moves twice.
    assert_eq!(hoist(&shared, |_| true).unwrap(), shared);
}

#[test]
fn a_call_that_reads_the_item_stays_in_the_branches() {
    let mut ops = loads_per_item(Next::jump(2));
    ops[1] = call("load", &[ITEM], MODEL, Next::jump(2));
    let wf = workflow(ops);
    assert_eq!(hoist(&wf, |_| true).unwrap(), wf);
}

#[test]
fn a_call_not_every_branch_makes_stays_in_the_branches() {
    // `if item { load } ...`: op 0's branches go to op 1 (load) or straight to op 2.
    let mut ops = loads_per_item(Next::jump(2));
    ops[0] = fan_out(Next::jump(4));
    ops.push(Operation::new(
        Vec::new(),
        Vec::new(),
        None,
        Next::branch(ITEM, 1, 2),
    ));
    let wf = workflow(ops);
    assert_eq!(hoist(&wf, |_| true).unwrap(), wf);

    // After a `return` that drops some items, too.
    let mut ops = loads_per_item(Next::jump(2));
    ops[0] = fan_out(Next::jump(4));
    ops.push(Operation::new(
        Vec::new(),
        Vec::new(),
        None,
        Next::branch(ITEM, 1, 5),
    ));
    ops.push(Operation::new(
        Vec::new(),
        Vec::new(),
        None,
        Next::return_unit(),
    ));
    let wf = workflow(ops);
    assert_eq!(hoist(&wf, |_| true).unwrap(), wf);
}

#[test]
fn a_task_that_is_not_shareable_is_called_per_item() {
    let wf = workflow(loads_per_item(Next::jump(2)));
    assert_eq!(hoist(&wf, |task| task != "load").unwrap(), wf);
}

#[test]
fn a_call_that_catches_its_failure_stays_in_the_branches() {
    let mut ops = loads_per_item(Next::jump(2));
    let Some(Action::Call(load)) = &mut ops[1].action else {
        unreachable!()
    };
    load.catch = Some(crate::ir::Catch { error: 9, next: 3 });
    let wf = workflow(ops);
    assert_eq!(hoist(&wf, |_| true).unwrap(), wf);
}
