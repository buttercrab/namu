//! The kernel: interprets a workflow's IR along one context.
//!
//! It applies literals and phis and follows control flow, and stops at the next action its caller
//! must perform: a task call, a fan-out, a collect, or a return. It knows nothing about where
//! values live or which machine runs what; values are read and written through [`ValueStore`].

use std::future::Future;

use anyhow::Context;
use namu_core::ir::{Next, Operation, Workflow};
use namu_core::{OpId, ValueId};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ids::NodeId;

/// The value tree as the kernel sees it.
pub trait ValueStore: Sync {
    /// Reads `value` as visible from `ctx`: defined there or in one of its ancestors.
    fn get(
        &self,
        ctx: NodeId,
        value: ValueId,
    ) -> impl Future<Output = anyhow::Result<Value>> + Send;

    /// Creates a child of `ctx` holding `values`, and returns it.
    fn put(
        &self,
        ctx: NodeId,
        values: Vec<(ValueId, Value)>,
    ) -> impl Future<Output = anyhow::Result<NodeId>> + Send;
}

/// Where to continue in the IR.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Start {
    /// Enter `op`, arriving from `pred` (which selects the phi sources).
    At { op: OpId, pred: Option<OpId> },
    /// Leave `op`, whose action is done, through its `next`.
    After { op: OpId },
}

/// The action that stopped the kernel. `ctx` is the context to perform it in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plan {
    Call {
        op: OpId,
        ctx: NodeId,
    },
    FanOut {
        op: OpId,
        ctx: NodeId,
    },
    Collect {
        op: OpId,
        ctx: NodeId,
    },
    /// The workflow returned, with `value` if it returns one.
    Return {
        ctx: NodeId,
        value: Option<ValueId>,
    },
}

/// Advances `ctx` from `start` to the next action.
pub async fn drive<S: ValueStore>(
    workflow: &Workflow,
    store: &S,
    mut ctx: NodeId,
    start: Start,
) -> anyhow::Result<Plan> {
    let (mut op_id, mut pred) = match start {
        Start::At { op, pred } => (op, pred),
        Start::After { op } => match leave(workflow, store, ctx, op).await? {
            Some(next) => (next, Some(op)),
            None => return Ok(returned(workflow, ctx, op)),
        },
    };
    loop {
        let op = operation(workflow, op_id)?;
        ctx = enter(store, ctx, op, pred).await?;
        if op.call.is_some() {
            return Ok(Plan::Call { op: op_id, ctx });
        }
        if op.fan_out.is_some() {
            return Ok(Plan::FanOut { op: op_id, ctx });
        }
        if op.collect.is_some() {
            return Ok(Plan::Collect { op: op_id, ctx });
        }
        match leave(workflow, store, ctx, op_id).await? {
            Some(next) => {
                pred = Some(op_id);
                op_id = next;
            }
            None => return Ok(returned(workflow, ctx, op_id)),
        }
    }
}

pub fn operation(workflow: &Workflow, op: OpId) -> anyhow::Result<&Operation> {
    workflow
        .operations
        .get(op)
        .with_context(|| format!("workflow {} has no op {op}", workflow.name))
}

/// Applies an operation's literals and phis as one step, in one new node. Every phi source is
/// read before anything is written, so phis are a parallel assignment (`a, b = b, a` works).
async fn enter<S: ValueStore>(
    store: &S,
    ctx: NodeId,
    op: &Operation,
    pred: Option<OpId>,
) -> anyhow::Result<NodeId> {
    let mut values = Vec::with_capacity(op.literals.len() + op.phis.len());
    for literal in &op.literals {
        let value = serde_json::from_str(&literal.value)
            .with_context(|| format!("literal {:?} is not JSON", literal.value))?;
        values.push((literal.output, value));
    }
    if let Some(pred) = pred {
        for phi in &op.phis {
            let (_, source) = phi
                .from
                .iter()
                .find(|(from, _)| *from == pred)
                .with_context(|| format!("phi {} has no source for op {pred}", phi.output))?;
            values.push((phi.output, store.get(ctx, *source).await?));
        }
    }
    if values.is_empty() {
        return Ok(ctx);
    }
    store.put(ctx, values).await
}

/// The op that follows `op`, or `None` if it returns.
pub async fn leave<S: ValueStore>(
    workflow: &Workflow,
    store: &S,
    ctx: NodeId,
    op: OpId,
) -> anyhow::Result<Option<OpId>> {
    Ok(match &operation(workflow, op)?.next {
        Next::Jump { next } => Some(*next),
        Next::Branch {
            var,
            true_next,
            false_next,
        } => {
            let cond = store.get(ctx, *var).await?;
            let cond = cond
                .as_bool()
                .with_context(|| format!("branch condition {var} is {cond}, not a bool"))?;
            Some(if cond { *true_next } else { *false_next })
        }
        Next::Return { .. } => None,
    })
}

fn returned(workflow: &Workflow, ctx: NodeId, op: OpId) -> Plan {
    let value = match workflow.operations[op].next {
        Next::Return { var } => var,
        _ => None,
    };
    Plan::Return { ctx, value }
}
