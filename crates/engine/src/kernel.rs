use async_trait::async_trait;
use namu_core::ir::{Call, Next, Operation, Workflow};
use namu_core::{ContextId, Value, ValueId};

#[derive(Debug, Clone)]
pub struct CallSpec {
    pub task_id: String,
    pub inputs: Vec<ValueId>,
    pub outputs: Vec<ValueId>,
}

#[derive(Debug, Clone)]
pub enum KernelAction {
    Dispatch {
        op_id: usize,
        ctx_id: ContextId,
        call: CallSpec,
    },
    Return {
        ctx_id: ContextId,
        return_var: Option<ValueId>,
    },
}

#[async_trait]
pub trait ValueStore: Send + Sync {
    type Value: Clone + Send + Sync + 'static;

    async fn get_value(&self, ctx_id: ContextId, val_id: ValueId) -> anyhow::Result<Self::Value>;
    async fn get_values(
        &self,
        ctx_id: ContextId,
        val_ids: &[ValueId],
    ) -> anyhow::Result<Vec<Self::Value>>;
    async fn set_value(
        &self,
        ctx_id: ContextId,
        val_id: ValueId,
        value: Self::Value,
    ) -> anyhow::Result<ContextId>;
}

pub trait ValueCodec: Send + Sync + Clone + 'static {
    type Value: Clone + Send + Sync + 'static;

    fn parse_literal(&self, raw: &str) -> anyhow::Result<Self::Value>;
    fn as_bool(&self, value: &Self::Value) -> anyhow::Result<bool>;
}

#[derive(Clone, Default)]
pub struct EngineKernel<C: ValueCodec> {
    codec: C,
}

impl<C: ValueCodec> EngineKernel<C> {
    pub fn new(codec: C) -> Self {
        Self { codec }
    }

    pub async fn drive_until_action<S: ValueStore<Value = C::Value>>(
        &self,
        workflow: &Workflow,
        store: &S,
        mut ctx_id: ContextId,
        mut op_id: usize,
        mut pred_op: Option<usize>,
    ) -> anyhow::Result<KernelAction> {
        loop {
            let operation = workflow
                .operations
                .get(op_id)
                .ok_or_else(|| anyhow::anyhow!("invalid op id {op_id}"))?;

            ctx_id = self.apply_literals(store, ctx_id, operation).await?;
            ctx_id = self.apply_phis(store, ctx_id, operation, pred_op).await?;

            if let Some(call) = &operation.call {
                return Ok(KernelAction::Dispatch {
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
                    return Ok(KernelAction::Return {
                        ctx_id,
                        return_var: return_var(&operation.next),
                    });
                }
            }
        }
    }

    pub async fn resolve_next<S: ValueStore<Value = C::Value>>(
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
                let cond_bool = self.codec.as_bool(&cond)?;
                Ok(Some(if cond_bool { *true_next } else { *false_next }))
            }
            Next::Return { .. } => Ok(None),
        }
    }

    async fn apply_literals<S: ValueStore<Value = C::Value>>(
        &self,
        store: &S,
        mut ctx_id: ContextId,
        op: &Operation,
    ) -> anyhow::Result<ContextId> {
        for lit in &op.literals {
            let value = self.codec.parse_literal(&lit.value)?;
            ctx_id = store.set_value(ctx_id, lit.output, value).await?;
        }
        Ok(ctx_id)
    }

    async fn apply_phis<S: ValueStore<Value = C::Value>>(
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

#[derive(Clone, Default)]
pub struct CoreValueCodec;

impl ValueCodec for CoreValueCodec {
    type Value = Value;

    fn parse_literal(&self, raw: &str) -> anyhow::Result<Self::Value> {
        let val = match raw {
            "true" => Value::new(true),
            "false" => Value::new(false),
            "()" => Value::new(()),
            _ => {
                if let Ok(n) = raw.parse::<i32>() {
                    Value::new(n)
                } else {
                    Value::new(raw.trim_matches('"').to_string())
                }
            }
        };
        Ok(val)
    }

    fn as_bool(&self, value: &Self::Value) -> anyhow::Result<bool> {
        value
            .downcast_ref::<bool>()
            .copied()
            .ok_or_else(|| anyhow::anyhow!("branch value not bool"))
    }
}

#[derive(Clone, Default)]
pub struct JsonCodec;

impl ValueCodec for JsonCodec {
    type Value = serde_json::Value;

    fn parse_literal(&self, raw: &str) -> anyhow::Result<Self::Value> {
        let val = match raw {
            "true" => serde_json::Value::Bool(true),
            "false" => serde_json::Value::Bool(false),
            "()" => serde_json::Value::Null,
            _ => {
                if let Ok(n) = raw.parse::<i32>() {
                    serde_json::Value::Number(n.into())
                } else {
                    serde_json::Value::String(raw.trim_matches('"').to_string())
                }
            }
        };
        Ok(val)
    }

    fn as_bool(&self, value: &Self::Value) -> anyhow::Result<bool> {
        value
            .as_bool()
            .ok_or_else(|| anyhow::anyhow!("branch value not bool"))
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
