use namu_core::ir::{Call, Next, Operation, Workflow};
use namu_core::{ContextId, ValueId};

use super::plan::{CallSpec, KernelPlan};
use crate::runtime::codec::ValueRuntime;
use crate::runtime::store::ValueStore;

#[derive(Clone, Default)]
pub struct EngineKernel<R: ValueRuntime> {
    runtime: R,
}

impl<R: ValueRuntime> EngineKernel<R> {
    pub fn new(runtime: R) -> Self {
        Self { runtime }
    }

    pub async fn drive_until_action<S: ValueStore<Value = R::Value>>(
        &self,
        workflow: &Workflow,
        store: &S,
        mut ctx_id: ContextId,
        mut op_id: usize,
        mut pred_op: Option<usize>,
    ) -> anyhow::Result<KernelPlan> {
        loop {
            let operation = workflow
                .operations
                .get(op_id)
                .ok_or_else(|| anyhow::anyhow!("invalid op id {op_id}"))?;

            ctx_id = self
                .apply_literals_and_phis(store, ctx_id, operation, pred_op)
                .await?;

            if let Some(call) = &operation.call {
                return Ok(KernelPlan::Dispatch {
                    op_id,
                    ctx_id,
                    call: call_spec(call),
                });
            }
            if let Some(fan_out) = &operation.fan_out {
                return Ok(KernelPlan::FanOut {
                    op_id,
                    ctx_id,
                    fan_out: fan_out.clone(),
                });
            }
            if let Some(collect) = &operation.collect {
                return Ok(KernelPlan::Collect {
                    op_id,
                    ctx_id,
                    collect: collect.clone(),
                });
            }

            match self.resolve_next(store, ctx_id, &operation.next).await? {
                Some(next) => {
                    pred_op = Some(op_id);
                    op_id = next;
                }
                None => {
                    return Ok(KernelPlan::Return {
                        ctx_id,
                        return_var: return_var(&operation.next),
                    });
                }
            }
        }
    }

    pub async fn resolve_next<S: ValueStore<Value = R::Value>>(
        &self,
        store: &S,
        ctx_id: ContextId,
        next: &Next,
    ) -> anyhow::Result<Option<usize>> {
        match next {
            Next::Jump { next } => Ok(Some(*next)),
            Next::Branch {
                var,
                true_next,
                false_next,
            } => {
                let cond = store.get_value(ctx_id, *var).await?;
                let cond_bool = self.runtime.as_bool(&cond)?;
                Ok(Some(if cond_bool { *true_next } else { *false_next }))
            }
            Next::Return { .. } => Ok(None),
        }
    }

    /// Applies an operation's literals and phis as one step. Every phi source is read from the
    /// incoming context before anything is written, so phis behave as a parallel assignment
    /// (`a, b = b, a` works). All values land in a single `set_values` call, which lets
    /// immutable stores create one node per step.
    async fn apply_literals_and_phis<S: ValueStore<Value = R::Value>>(
        &self,
        store: &S,
        ctx_id: ContextId,
        op: &Operation,
        pred_op: Option<usize>,
    ) -> anyhow::Result<ContextId> {
        let mut values = Vec::with_capacity(op.literals.len() + op.phis.len());
        for lit in &op.literals {
            values.push((lit.output, self.runtime.parse_literal(&lit.value)?));
        }
        if let Some(pred_op) = pred_op {
            for phi in &op.phis {
                let entry = phi
                    .from
                    .iter()
                    .find(|(from_op, _)| *from_op == pred_op)
                    .ok_or_else(|| anyhow::anyhow!("phi missing predecessor"))?;
                values.push((phi.output, store.get_value(ctx_id, entry.1).await?));
            }
        }
        if values.is_empty() {
            return Ok(ctx_id);
        }
        store.set_values(ctx_id, values).await
    }
}

fn call_spec(call: &Call) -> CallSpec {
    CallSpec {
        task_id: call.task_id.clone(),
        inputs: call.inputs.clone(),
        outputs: call.outputs.clone(),
    }
}

fn return_var(next: &Next) -> Option<ValueId> {
    match next {
        Next::Return { var } => *var,
        _ => None,
    }
}
