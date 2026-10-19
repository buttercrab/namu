use serde::{Deserialize, Serialize};

use crate::{OpId, ValueId};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Workflow {
    pub name: String,
    pub operations: Vec<Operation>,
}

impl Workflow {
    pub fn new(name: String, operations: Vec<Operation>) -> Self {
        Self { name, operations }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Literal {
    pub output: ValueId,
    /// The value as JSON text, so the IR stays readable to people and to the tools that write it.
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Phi {
    pub output: ValueId,
    /// Pairs of (predecessor operation, the value coming from it).
    pub from: Vec<(OpId, ValueId)>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Call {
    pub task_id: String,
    pub inputs: Vec<ValueId>,
    pub outputs: Vec<ValueId>,
    /// Where control goes if the task fails. Without it, a failure fails the branch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catch: Option<Catch>,
}

/// A call's error edge, as LLVM's `invoke` has one: if the task fails, its outputs are not
/// defined; its error message is bound to `error`, and control goes to `next` instead of the
/// call's own `next`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Catch {
    pub error: ValueId,
    pub next: OpId,
}

/// Fan-out: every element of the list `input` continues in its own child context, where it is
/// bound to `output`. The context executing this operation becomes the scope that a later
/// [`Collect`] closes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FanOut {
    pub input: ValueId,
    pub output: ValueId,
}

/// Fan-in: gathers `input` from every surviving branch of the innermost fan-out scope into a list
/// bound to `output`, then continues once in the scope's context.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Collect {
    pub input: ValueId,
    pub output: ValueId,
}

/// What an operation does after its literals and phis.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Action {
    Call(Call),
    FanOut(FanOut),
    Collect(Collect),
}

impl Action {
    pub fn inputs(&self) -> &[ValueId] {
        match self {
            Action::Call(call) => &call.inputs,
            Action::FanOut(fan_out) => std::slice::from_ref(&fan_out.input),
            Action::Collect(collect) => std::slice::from_ref(&collect.input),
        }
    }

    pub fn outputs(&self) -> &[ValueId] {
        match self {
            Action::Call(call) => &call.outputs,
            Action::FanOut(fan_out) => std::slice::from_ref(&fan_out.output),
            Action::Collect(collect) => std::slice::from_ref(&collect.output),
        }
    }
}

/// One step of a workflow: literals, then phis (applied together), then at most one action, then
/// a jump, a branch or a return.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Operation {
    pub literals: Vec<Literal>,
    pub phis: Vec<Phi>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<Action>,
    pub next: Next,
}

impl Operation {
    pub fn new(literals: Vec<Literal>, phis: Vec<Phi>, action: Option<Action>, next: Next) -> Self {
        Self {
            literals,
            phis,
            action,
            next,
        }
    }

    pub fn call(&self) -> Option<&Call> {
        match &self.action {
            Some(Action::Call(call)) => Some(call),
            _ => None,
        }
    }

    pub fn fan_out(&self) -> Option<&FanOut> {
        match &self.action {
            Some(Action::FanOut(fan_out)) => Some(fan_out),
            _ => None,
        }
    }

    pub fn collect(&self) -> Option<&Collect> {
        match &self.action {
            Some(Action::Collect(collect)) => Some(collect),
            _ => None,
        }
    }

    /// The error edge of its call, if it has one.
    pub fn catch(&self) -> Option<&Catch> {
        self.call()?.catch.as_ref()
    }

    /// The operations control can go to next: through `next`, or through a call's error edge.
    pub fn successors(&self) -> impl Iterator<Item = OpId> {
        self.next
            .successors()
            .chain(self.catch().map(|catch| catch.next))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Next {
    Jump {
        next: OpId,
    },
    Branch {
        var: ValueId,
        true_next: OpId,
        false_next: OpId,
    },
    Return {
        var: Option<ValueId>,
    },
}

impl Next {
    pub fn jump(next: OpId) -> Self {
        Self::Jump { next }
    }

    pub fn branch(var: ValueId, true_next: OpId, false_next: OpId) -> Self {
        Self::Branch {
            var,
            true_next,
            false_next,
        }
    }

    pub fn return_value(var: ValueId) -> Self {
        Self::Return { var: Some(var) }
    }

    pub fn return_unit() -> Self {
        Self::Return { var: None }
    }

    /// The operations control can go to next.
    pub fn successors(&self) -> impl Iterator<Item = OpId> {
        let (first, second) = match *self {
            Next::Jump { next } => (Some(next), None),
            Next::Branch {
                true_next,
                false_next,
                ..
            } => (Some(true_next), Some(false_next)),
            Next::Return { .. } => (None, None),
        };
        first.into_iter().chain(second)
    }
}
