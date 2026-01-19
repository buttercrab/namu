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

            ctx_id = self.apply_literals(store, ctx_id, operation).await?;
            ctx_id = self.apply_phis(store, ctx_id, operation, pred_op).await?;

            if let Some(call) = &operation.call {
                return Ok(KernelPlan::Dispatch {
                    op_id,
                    ctx_id,
                    call: call_spec(call),
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

    async fn apply_literals<S: ValueStore<Value = R::Value>>(
        &self,
        store: &S,
        mut ctx_id: ContextId,
        op: &Operation,
    ) -> anyhow::Result<ContextId> {
        for lit in &op.literals {
            let value = self.runtime.parse_literal(&lit.value)?;
            ctx_id = store.set_value(ctx_id, lit.output, value).await?;
        }
        Ok(ctx_id)
    }

    async fn apply_phis<S: ValueStore<Value = R::Value>>(
        &self,
        store: &S,
        mut ctx_id: ContextId,
        op: &Operation,
        pred_op: Option<usize>,
    ) -> anyhow::Result<ContextId> {
        let Some(pred_op) = pred_op else {
            return Ok(ctx_id);
        };
        for phi in &op.phis {
            let entry = phi
                .from
                .iter()
                .find(|(from_op, _)| *from_op == pred_op)
                .ok_or_else(|| anyhow::anyhow!("phi missing predecessor"))?;
            let val_id = entry.1;
            let val = store.get_value(ctx_id, val_id).await?;
            ctx_id = store.set_value(ctx_id, phi.output, val).await?;
        }
        Ok(ctx_id)
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
