use super::*;
use crate::ir::{Collect, FanOut};

const ITEMS: ValueId = 0;
const ITEM: ValueId = 1;
const PATH: ValueId = 2;
const MODEL: ValueId = 3;
const LABEL: ValueId = 4;
const LABELS: ValueId = 5;

const SHAREABLE: Task = Task {
    shareable: true,
    stream: false,
};

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
    let shared = hoist(&wf, |_| SHAREABLE).unwrap();
    assert_eq!(tasks(&shared), [Some("load"), None, Some("classify"), None]);
    assert!(shared.operations[1].fan_out().is_some());
    assert_eq!(shared.operations[0].next, Next::jump(1));
    assert_eq!(shared.operations[1].next, Next::jump(2));
    // Nothing moves twice.
    assert_eq!(hoist(&shared, |_| SHAREABLE).unwrap(), shared);
}

#[test]
fn a_call_that_reads_the_item_stays_in_the_branches() {
    let mut ops = loads_per_item(Next::jump(2));
    ops[1] = call("load", &[ITEM], MODEL, Next::jump(2));
    let wf = workflow(ops);
    assert_eq!(hoist(&wf, |_| SHAREABLE).unwrap(), wf);
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
    assert_eq!(hoist(&wf, |_| SHAREABLE).unwrap(), wf);

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
    assert_eq!(hoist(&wf, |_| SHAREABLE).unwrap(), wf);
}

#[test]
fn a_task_that_is_not_shareable_is_called_per_item() {
    let wf = workflow(loads_per_item(Next::jump(2)));
    assert_eq!(
        hoist(&wf, |task| Task {
            shareable: task != "load",
            stream: false,
        })
        .unwrap(),
        wf
    );
}

#[test]
fn a_call_that_catches_its_failure_stays_in_the_branches() {
    let mut ops = loads_per_item(Next::jump(2));
    let Some(Action::Call(load)) = &mut ops[1].action else {
        unreachable!()
    };
    load.catch = Some(crate::ir::Catch { error: 9, next: 3 });
    let wf = workflow(ops);
    assert_eq!(hoist(&wf, |_| SHAREABLE).unwrap(), wf);
}

/// `classify(load(path), item)` for every item of a list `make_items` makes, at op 0.
fn loads_per_item_made() -> Workflow {
    let mut ops = loads_per_item(Next::jump(3));
    ops.insert(0, call("make_items", &[], ITEMS, Next::jump(1)));
    for op in &mut ops[1..] {
        if let Next::Jump { next } = &mut op.next {
            *next += 1;
        }
    }
    workflow(ops)
}

#[test]
fn a_shared_call_goes_after_the_call_that_makes_the_list() {
    // The list is made first: if that fails, the shared call is never made.
    let shared = hoist(&loads_per_item_made(), |_| SHAREABLE).unwrap();
    assert_eq!(
        tasks(&shared),
        [
            Some("make_items"),
            Some("load"),
            None,
            Some("classify"),
            None
        ]
    );
    assert!(shared.operations[2].fan_out().is_some());
}

#[test]
fn a_shared_call_goes_before_the_stream_that_feeds_the_fan_out() {
    // Before the stream, which must stay next to its fan-out to feed it item by item.
    let shared = hoist(&loads_per_item_made(), |task| Task {
        shareable: task != "make_items",
        stream: task == "make_items",
    })
    .unwrap();
    assert_eq!(
        tasks(&shared),
        [
            Some("load"),
            Some("make_items"),
            None,
            Some("classify"),
            None
        ]
    );
    assert_eq!(shared.operations[1].next, Next::jump(2));
    assert!(shared.operations[2].fan_out().is_some());
}
