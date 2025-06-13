use serde::{Deserialize, Serialize};

pub type VarId = usize;
pub type OpId = usize;

#[derive(Debug, Serialize, Deserialize)]
pub struct Workflow {
    pub name: String,
    pub operations: Vec<Operation>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Literal {
    pub value: String,
    pub output: VarId,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Task {
    pub name: String,
    pub inputs: Vec<VarId>,
    pub output: VarId,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Operation {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub phis: Vec<Phi>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub literals: Vec<Literal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<Task>,
    pub next: Next,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Phi {
    pub id: VarId,
    pub from: Vec<(OpId, VarId)>,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Next {
    Jump {
        next: OpId,
    },
    Branch {
        var: VarId,
        true_next: OpId,
        false_next: OpId,
    },
    Return {
        var: Option<VarId>,
    },
}
