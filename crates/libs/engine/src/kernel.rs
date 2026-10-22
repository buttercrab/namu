//! The kernel: interprets a workflow's IR along one context.
//!
//! It applies literals and phis, evaluates built-in expressions, and follows control flow, and
//! stops at the next action its caller
//! must perform: a task call, a fan-out, a collect, or a return. It knows nothing about where
//! values live or which machine runs what; values are read and written through [`ValueStore`].

use std::future::Future;

use anyhow::Context;
use namu_core::ir::{Action, Call, Collect, FanOut, Next, Operation, Workflow};
use namu_core::value::Value;
use namu_core::{OpId, ValueId, builtin};
use serde::{Deserialize, Serialize};

/// A workflow's literals as values, decoded once per run. The IR keeps them as JSON text, so it
/// stays readable to people and to tools that write it.
#[derive(Debug)]
pub struct Literals(Vec<Vec<Value>>);

impl Literals {
    pub fn decode(workflow: &Workflow) -> anyhow::Result<Self> {
        let ops = workflow.operations.iter().map(|op| {
            op.literals
                .iter()
                .map(|literal| {
                    let mut bytes = Vec::new();
                    let mut json = serde_json::Deserializer::from_str(&literal.value);
                    let mut msgpack = rmp_serde::Serializer::new(&mut bytes).with_struct_map();
                    serde_transcode::transcode(&mut json, &mut msgpack)
                        .with_context(|| format!("literal {:?} is not JSON", literal.value))?;
                    Ok(Value::encoded(bytes))
                })
                .collect::<anyhow::Result<Vec<_>>>()
        });
        Ok(Self(ops.collect::<anyhow::Result<_>>()?))
    }
}

/// The value tree as the kernel sees it.
pub trait ValueStore: Sync {
    /// A context: a point in the tree, which keeps what it reads from alive.
    type Ctx: Send + Sync;

    /// Reads `value` as visible from `ctx`: defined there or in one of its ancestors.
    fn get(
        &self,
        ctx: &Self::Ctx,
        value: ValueId,
    ) -> impl Future<Output = anyhow::Result<Value>> + Send;

    /// Grows `ctx` by a node holding `values`, created at point `at`, and returns it.
    fn put(
        &self,
        ctx: &Self::Ctx,
        values: Vec<(ValueId, Value)>,
        at: Point,
    ) -> impl Future<Output = anyhow::Result<Self::Ctx>> + Send;
}

/// Where in an op a node is created, which decides what is still live there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Point {
    /// After the op's literals and phis.
    Enter(OpId),
    /// After the op's action: a call's outputs, a fan-out's item, a collect's list.
    Action(OpId),
}

/// Where to continue in the IR.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Start {
    /// Enter `op`, arriving from `pred` (which selects the phi sources).
    At { op: OpId, pred: Option<OpId> },
    /// Leave `op`, whose action is done, through its `next`.
    After { op: OpId },
}

/// The action that stopped the kernel, at op `op`. `ctx` is the context to perform it in.
#[derive(Debug)]
pub enum Plan<'w, C> {
    Call {
        op: OpId,
        call: &'w Call,
        ctx: C,
    },
    FanOut {
        op: OpId,
        fan_out: &'w FanOut,
        ctx: C,
    },
    Collect {
        op: OpId,
        collect: &'w Collect,
        ctx: C,
    },
    /// The workflow returned, with `value` if it returns one.
    Return {
        ctx: C,
        value: Option<ValueId>,
    },
}

/// Advances `ctx` from `start` to the next action.
pub async fn drive<'w, S: ValueStore>(
    workflow: &'w Workflow,
    literals: &Literals,
    store: &S,
    mut ctx: S::Ctx,
    start: Start,
) -> anyhow::Result<Plan<'w, S::Ctx>> {
    let (mut op_id, mut pred) = match start {
        Start::At { op, pred } => (op, pred),
        Start::After { op } => match leave(workflow, store, &ctx, op).await? {
            Some(next) => (next, Some(op)),
            None => return Ok(returned(workflow, ctx, op)),
        },
    };
    loop {
        let op = operation(workflow, op_id)?;
        ctx = enter(store, ctx, op_id, op, &literals.0[op_id], pred).await?;
        match &op.action {
            Some(Action::Call(call)) => {
                return Ok(Plan::Call {
                    op: op_id,
                    call,
                    ctx,
                });
            }
            Some(Action::FanOut(fan_out)) => {
                return Ok(Plan::FanOut {
                    op: op_id,
                    fan_out,
                    ctx,
                });
            }
            Some(Action::Collect(collect)) => {
                return Ok(Plan::Collect {
                    op: op_id,
                    collect,
                    ctx,
                });
            }
            None => {}
        }
        match leave(workflow, store, &ctx, op_id).await? {
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

/// Applies an operation's literals and phis, then evaluates its built-ins in order, as one step in
/// one new node. Every phi source is read before anything is written, so phis are a parallel
/// assignment (`a, b = b, a` works); a built-in reads what the step made before it, or the tree.
async fn enter<S: ValueStore>(
    store: &S,
    ctx: S::Ctx,
    op_id: OpId,
    op: &Operation,
    literals: &[Value],
    pred: Option<OpId>,
) -> anyhow::Result<S::Ctx> {
    let mut values = Vec::with_capacity(op.literals.len() + op.phis.len());
    for (literal, value) in op.literals.iter().zip(literals) {
        values.push((literal.output, value.clone()));
    }
    if let Some(pred) = pred {
        for phi in &op.phis {
            let (_, source) = phi
                .from
                .iter()
                .find(|(from, _)| *from == pred)
                .with_context(|| format!("phi {} has no source for op {pred}", phi.output))?;
            values.push((phi.output, store.get(&ctx, *source).await?));
        }
    }
    for builtin in &op.builtins {
        let mut inputs = Vec::with_capacity(builtin.inputs.len());
        for &input in &builtin.inputs {
            // Steps are small: a scan beats a map.
            inputs.push(match values.iter().rfind(|(made, _)| *made == input) {
                Some((_, value)) => value.clone(),
                None => store.get(&ctx, input).await?,
            });
        }
        let value = builtin::eval(builtin.op, builtin.ty, &inputs)
            .with_context(|| format!("built-in {:?} at op {op_id}", builtin.op))?;
        values.push((builtin.output, value));
    }
    if values.is_empty() {
        return Ok(ctx);
    }
    store.put(&ctx, values, Point::Enter(op_id)).await
}

/// The op that follows `op`, or `None` if it returns.
pub async fn leave<S: ValueStore>(
    workflow: &Workflow,
    store: &S,
    ctx: &S::Ctx,
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
                .with_context(|| format!("branch condition {var}"))?;
            Some(if cond { *true_next } else { *false_next })
        }
        Next::Return { .. } => None,
    })
}

fn returned<C>(workflow: &Workflow, ctx: C, op: OpId) -> Plan<'_, C> {
    let value = match workflow.operations[op].next {
        Next::Return { var } => var,
        _ => None,
    };
    Plan::Return { ctx, value }
}
