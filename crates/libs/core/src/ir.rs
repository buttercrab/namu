use serde::{Deserialize, Serialize};

use crate::types::{Defs, Type};
use crate::{OpId, ValueId};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Workflow {
    pub name: String,
    pub operations: Vec<Operation>,
    /// Every value's type, by value id; empty in a workflow written without them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub types: Vec<Type>,
    /// The shapes of the named types in `types`.
    #[serde(default, skip_serializing_if = "Defs::is_empty")]
    pub defs: Defs,
}

impl Workflow {
    pub fn new(name: String, operations: Vec<Operation>) -> Self {
        Self {
            name,
            operations,
            types: Vec::new(),
            defs: Defs::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Literal {
    pub output: ValueId,
    /// The value as JSON text, so the IR stays readable to people and to the tools that write it.
    pub value: String,
}

/// An expression the engine evaluates itself, where it stands: arithmetic, a comparison, logic.
/// `ty` is its operands' type, which says how to read them; see [`crate::builtin`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Builtin {
    pub op: BuiltinOp,
    pub ty: ScalarType,
    pub inputs: Vec<ValueId>,
    pub output: ValueId,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum BuiltinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Neg,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    Not,
}

/// The types built-ins work on.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ScalarType {
    Bool,
    I32,
    I64,
    U32,
    U64,
    F32,
    F64,
    String,
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
/// bound to `output`, then continues once in the scope's context. With `reduce`, it folds them
/// instead, in branch order, as they end: `output` is the last accumulator.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Collect {
    pub input: ValueId,
    pub output: ValueId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reduce: Option<Reduce>,
}

/// Windowing: closes the innermost fan-out scope like a collect, but continues once per `size`
/// items rather than once for all of them: each window of items, in branch order, is a branch of
/// a new scope, bound to `output`. A later collect closes that scope.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Window {
    pub input: ValueId,
    pub output: ValueId,
    pub size: u64,
}

/// A fold: `task_id(accumulator, item)` gives the next accumulator, starting from `init`, a value
/// of the scope's own context.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Reduce {
    pub task_id: String,
    pub init: ValueId,
}

/// What an operation does after its literals and phis.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Action {
    Call(Call),
    FanOut(FanOut),
    Collect(Collect),
    Window(Window),
}

impl Action {
    /// The values it reads: a reduce's `init` too, which must outlive the scope it closes.
    pub fn inputs(&self) -> impl Iterator<Item = ValueId> + '_ {
        let (values, init): (&[ValueId], _) = match self {
            Action::Call(call) => (&call.inputs, None),
            Action::FanOut(fan_out) => (std::slice::from_ref(&fan_out.input), None),
            Action::Collect(collect) => (
                std::slice::from_ref(&collect.input),
                collect.reduce.as_ref().map(|reduce| reduce.init),
            ),
            Action::Window(window) => (std::slice::from_ref(&window.input), None),
        };
        values.iter().copied().chain(init)
    }

    pub fn outputs(&self) -> &[ValueId] {
        match self {
            Action::Call(call) => &call.outputs,
            Action::FanOut(fan_out) => std::slice::from_ref(&fan_out.output),
            Action::Collect(collect) => std::slice::from_ref(&collect.output),
            Action::Window(window) => std::slice::from_ref(&window.output),
        }
    }
}

/// One step of a workflow: literals, then phis (applied together), then built-ins (in order),
/// then at most one action, then a jump, a branch or a return.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Operation {
    pub literals: Vec<Literal>,
    pub phis: Vec<Phi>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub builtins: Vec<Builtin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<Action>,
    pub next: Next,
}

impl Operation {
    pub fn new(literals: Vec<Literal>, phis: Vec<Phi>, action: Option<Action>, next: Next) -> Self {
        Self {
            literals,
            phis,
            builtins: Vec::new(),
            action,
            next,
        }
    }

    pub fn with_builtins(mut self, builtins: Vec<Builtin>) -> Self {
        self.builtins = builtins;
        self
    }

    /// Whether control enters it without computing anything first: no literals, phis or
    /// built-ins.
    pub fn entered_directly(&self) -> bool {
        self.literals.is_empty() && self.phis.is_empty() && self.builtins.is_empty()
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

    pub fn window(&self) -> Option<&Window> {
        match &self.action {
            Some(Action::Window(window)) => Some(window),
            _ => None,
        }
    }

    /// The task it uses: the one it calls, or the one a reducing collect folds with.
    pub fn task_id(&self) -> Option<&str> {
        match &self.action {
            Some(Action::Call(call)) => Some(&call.task_id),
            Some(Action::Collect(Collect {
                reduce: Some(reduce),
                ..
            })) => Some(&reduce.task_id),
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
