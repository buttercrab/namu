use namu_core::{ContextId, ValueId};

#[derive(Debug, Clone)]
pub struct CallSpec {
    pub task_id: String,
    pub inputs: Vec<ValueId>,
    pub outputs: Vec<ValueId>,
}

#[derive(Debug, Clone)]
pub enum KernelPlan {
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

pub type KernelAction = KernelPlan;
