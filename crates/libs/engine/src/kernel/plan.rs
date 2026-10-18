use namu_core::ir::{Collect, FanOut};
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
    /// Fan out over a list value: one child context per element.
    FanOut {
        op_id: usize,
        ctx_id: ContextId,
        fan_out: FanOut,
    },
    /// This branch reached a fan-in point of its innermost scope.
    Collect {
        op_id: usize,
        ctx_id: ContextId,
        collect: Collect,
    },
    Return {
        ctx_id: ContextId,
        return_var: Option<ValueId>,
    },
}

pub type KernelAction = KernelPlan;
