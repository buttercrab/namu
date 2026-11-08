//! Level-2 dedup (design §6.1): what every branch of a scope computes alike is computed once, by
//! the scope.
//!
//! A call or built-in inside a fan-out whose inputs all come from outside it, and that every
//! branch makes, gives every branch the same value. It is moved to just before the fan-out, as
//! if it had been written there: the scope's context holds the value, and every branch reads it
//! through its parent. Moved, it runs once even if there are no items, and if it fails, the
//! branch that fans out fails.
//!
//! It stays in the branches when
//! - a branch may not make it: it is behind an `if`, or after a `return` that drops the item;
//! - its task is not `shareable`: one with side effects (a checkpoint), each of whose calls counts,
//!   or a stream, which would no longer feed its fan-out item by item;
//! - it catches its own failure (`attempt`), which is the branch's control flow.
//!
//! Scopes are left one at a time until nothing moves, so a value lands at the depth of its
//! deepest input. A window's scope is left as it is.

use std::collections::{HashMap, HashSet};

use crate::ir::{Action, Builtin, Call, Next, Operation, Workflow};
use crate::scopes::{self, Scopes};
use crate::{OpId, ValueId};

/// What the pass knows of a task; a task it is not told of is neither.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Task {
    /// Whether one call of it may stand for every branch of a scope that makes it alike.
    pub shareable: bool,
    /// Whether it is a stream, which feeds the fan-out right after it item by item.
    pub stream: bool,
}

/// `workflow` with what its scopes' branches compute alike moved out of them, by what `task`
/// says of each task.
pub fn hoist(workflow: &Workflow, task: impl Fn(&str) -> Task) -> anyhow::Result<Workflow> {
    let mut workflow = workflow.clone();
    loop {
        let scopes = scopes::scopes(&workflow)?;
        let found = (0..workflow.operations.len())
            .find_map(|fan_out| Shared::find(&workflow, &scopes, fan_out, &task));
        let Some(shared) = found else {
            return Ok(workflow);
        };
        workflow = shared.apply(workflow, &task);
        debug_assert!(scopes::scopes(&workflow).is_ok());
    }
}

/// A built-in of an op, by position, or its call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Unit {
    Builtin(OpId, usize),
    Call(OpId),
}

/// What moves out of one fan-out's scope: in an order where each comes after what it reads.
struct Shared {
    fan_out: OpId,
    units: Vec<Unit>,
}

impl Shared {
    fn find(
        workflow: &Workflow,
        scopes: &Scopes,
        fan_out: OpId,
        task: &impl Fn(&str) -> Task,
    ) -> Option<Self> {
        let ops = &workflow.operations;
        ops[fan_out].fan_out()?;
        let made_in = scopes.made_in(workflow);
        let inside = |value: &ValueId| made_in.get(value).is_some_and(|s| s.contains(&fan_out));
        let region = Region {
            ops,
            scopes,
            fan_out,
        };
        // The ops of the scope itself (not of scopes nested in it) that every branch passes.
        let every: Vec<OpId> = (0..ops.len())
            .filter(|&op| region.innermost(op) && region.passed_by_every_branch(op))
            .collect();

        let mut units = Vec::new();
        let mut taken = HashSet::new();
        let mut shared: HashSet<ValueId> = HashSet::new();
        // Each round takes what reads only values from outside, or what the rounds before took.
        loop {
            let before = units.len();
            for &op_id in &every {
                let op = &ops[op_id];
                let builtins = op.builtins.iter().enumerate();
                let candidates = builtins
                    .map(|(i, builtin)| (Unit::Builtin(op_id, i), &builtin.inputs[..], true))
                    .chain(op.call().map(|call| {
                        let movable = call.catch.is_none() && task(&call.task_id).shareable;
                        (Unit::Call(op_id), &call.inputs[..], movable)
                    }));
                for (unit, inputs, movable) in candidates {
                    let ready = inputs.iter().all(|v| !inside(v) || shared.contains(v));
                    if movable && ready && taken.insert(unit) {
                        shared.extend(outputs(op, unit));
                        units.push(unit);
                    }
                }
            }
            if units.len() == before {
                break;
            }
        }
        (!units.is_empty()).then_some(Self { fan_out, units })
    }

    /// Moves the units to just before the fan-out, after the entered part of that op and before
    /// its action: before the stream that feeds it if it is fused with one, which must stay next
    /// to it, and after any other call that makes its list, whose failure must come first.
    fn apply(self, mut workflow: Workflow, task: &impl Fn(&str) -> Task) -> Workflow {
        let fused = scopes::fused_fan_outs(&workflow);
        let streams = |op: &OpId| {
            let call = workflow.operations[*op].call();
            fused[*op] == Some(self.fan_out) && call.is_some_and(|call| task(&call.task_id).stream)
        };
        let at = (0..fused.len()).find(streams).unwrap_or(self.fan_out);
        let ops = &mut workflow.operations;

        // Take the units out, in their order.
        let mut taken: Vec<Moved> = Vec::with_capacity(self.units.len());
        let mut builtins: HashMap<Unit, Builtin> = HashMap::new();
        let mut emptied = HashSet::new();
        for (op_id, op) in ops.iter_mut().enumerate() {
            let moving = |i: &usize| self.units.contains(&Unit::Builtin(op_id, *i));
            if !(0..op.builtins.len()).any(|i| moving(&i)) {
                continue;
            }
            for (i, builtin) in std::mem::take(&mut op.builtins).into_iter().enumerate() {
                if moving(&i) {
                    builtins.insert(Unit::Builtin(op_id, i), builtin);
                } else {
                    op.builtins.push(builtin);
                }
            }
            emptied.insert(op_id);
        }
        for &unit in &self.units {
            taken.push(match unit {
                Unit::Builtin(..) => Moved::Builtin(builtins.remove(&unit).unwrap()),
                Unit::Call(op_id) => {
                    emptied.insert(op_id);
                    let Some(Action::Call(call)) = ops[op_id].action.take() else {
                        unreachable!("op {op_id} calls");
                    };
                    Moved::Call(call)
                }
            });
        }
        let mut moved = into_ops(taken);

        // An op left with nothing to do, that jumps on, is passed over (unless a phi needs it as
        // the edge it arrives by).
        let dropped: Vec<bool> = (0..ops.len())
            .map(|op_id| {
                let op = &ops[op_id];
                let Next::Jump { next } = op.next else {
                    return false;
                };
                let arrives_by = ops[next].phis.iter().flat_map(|phi| &phi.from);
                emptied.contains(&op_id)
                    && op_id != at
                    && next != op_id
                    && op.entered_directly()
                    && op.action.is_none()
                    && !arrives_by.clone().any(|&(from, _)| from == op_id)
            })
            .collect();

        // New ids: the op at `at` splits into its entered part (if any), the moved ops, and its
        // action. Control enters an op at its first piece and leaves from its last.
        let split = !ops[at].entered_directly();
        let mut first = vec![0; ops.len()];
        let mut last = vec![0; ops.len()];
        let mut next_id = 0;
        for op_id in 0..ops.len() {
            if dropped[op_id] {
                continue;
            }
            first[op_id] = next_id;
            if op_id == at {
                next_id += usize::from(split) + moved.len();
            }
            last[op_id] = next_id;
            next_id += 1;
        }
        let target = |mut op_id: OpId| {
            while dropped[op_id] {
                let Next::Jump { next } = ops[op_id].next else {
                    unreachable!("only jumps are dropped");
                };
                op_id = next;
            }
            first[op_id]
        };

        let mut rewritten = Vec::with_capacity(next_id);
        for (op_id, op) in ops.iter().enumerate() {
            if dropped[op_id] {
                continue;
            }
            let mut op = op.clone();
            op.next = match op.next {
                Next::Jump { next } => Next::jump(target(next)),
                Next::Branch {
                    var,
                    true_next,
                    false_next,
                } => Next::branch(var, target(true_next), target(false_next)),
                done @ Next::Return { .. } => done,
            };
            if let Some(Action::Call(Call {
                catch: Some(catch), ..
            })) = &mut op.action
            {
                catch.next = target(catch.next);
            }
            for phi in &mut op.phis {
                for (from, _) in &mut phi.from {
                    *from = last[*from];
                }
            }
            if op_id == at {
                let action = Operation::new(Vec::new(), Vec::new(), op.action.take(), op.next);
                if split {
                    op.next = Next::jump(rewritten.len() + 1);
                    rewritten.push(op);
                }
                for mut moved in std::mem::take(&mut moved) {
                    moved.next = Next::jump(rewritten.len() + 1);
                    rewritten.push(moved);
                }
                rewritten.push(action);
            } else {
                rewritten.push(op);
            }
        }
        workflow.operations = rewritten;
        workflow
    }
}

enum Moved {
    Builtin(Builtin),
    Call(Call),
}

/// The moved units as ops, in order: each call with the built-ins before it; their `next` is set
/// where they are placed.
fn into_ops(moved: Vec<Moved>) -> Vec<Operation> {
    let mut ops = Vec::new();
    let mut builtins = Vec::new();
    for unit in moved {
        match unit {
            Moved::Builtin(builtin) => builtins.push(builtin),
            Moved::Call(call) => ops.push(
                Operation::new(
                    Vec::new(),
                    Vec::new(),
                    Some(Action::Call(call)),
                    Next::return_unit(),
                )
                .with_builtins(std::mem::take(&mut builtins)),
            ),
        }
    }
    if !builtins.is_empty() {
        ops.push(
            Operation::new(Vec::new(), Vec::new(), None, Next::return_unit())
                .with_builtins(builtins),
        );
    }
    ops
}

fn outputs(op: &Operation, unit: Unit) -> Vec<ValueId> {
    match unit {
        Unit::Builtin(_, i) => vec![op.builtins[i].output],
        Unit::Call(_) => op
            .call()
            .map(|call| call.outputs.clone())
            .unwrap_or_default(),
    }
}

/// The ops of one fan-out's scope, as its branches go through them.
struct Region<'w> {
    ops: &'w [Operation],
    scopes: &'w Scopes,
    fan_out: OpId,
}

impl Region<'_> {
    /// Whether `op` is in the scope itself, not in one nested in it.
    fn innermost(&self, op: OpId) -> bool {
        let entry = self.scopes.entry[op].as_ref();
        entry.and_then(|scopes| scopes.last()) == Some(&self.fan_out)
    }

    /// Whether every branch of the scope that goes on passes `op`: no path from the fan-out
    /// leaves the scope, or ends, without it. A branch that fails is not one that goes on.
    fn passed_by_every_branch(&self, op: OpId) -> bool {
        let mut seen = HashSet::new();
        let mut work: Vec<OpId> = self.ops[self.fan_out].successors().collect();
        let mut passed = false;
        while let Some(at) = work.pop() {
            if at == op {
                passed = true;
                continue;
            }
            if !seen.insert(at) {
                continue;
            }
            match self.onward(at) {
                Some(next) => work.extend(next),
                None => return false,
            }
        }
        passed
    }

    /// Where a branch at `at` goes on to in the scope, or `None` where it leaves it or ends. A
    /// scope nested in it is gone through to its collect, after which the branch goes on once.
    fn onward(&self, at: OpId) -> Option<Vec<OpId>> {
        let op = &self.ops[at];
        if matches!(op.next, Next::Return { .. })
            || self.scopes.collect_of[self.fan_out] == Some(at)
        {
            return None;
        }
        if op.fan_out().is_none() {
            return Some(op.successors().collect());
        }
        let mut closes = self.scopes.collect_of[at]?;
        while self.ops[closes].window().is_some() {
            closes = self.scopes.collect_of[closes]?;
        }
        Some(self.ops[closes].successors().collect())
    }
}

#[cfg(test)]
mod tests;
