use serde::{Deserialize, Serialize};

pub type ValueId = usize;
pub type OpId = usize;

#[derive(Debug, Serialize, Deserialize)]
pub struct Workflow {
    pub name: String,
    pub operations: Vec<Operation>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Literal {
    pub value: String,
    pub output: ValueId,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Call {
    pub name: String,
    pub inputs: Vec<ValueId>,
    pub output: ValueId,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Operation {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub phis: Vec<PhiNode>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub literals: Vec<Literal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<Call>,
    pub next: Next,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PhiNode {
    pub id: ValueId,
    pub from: Vec<(OpId, ValueId)>,
}

#[derive(Debug, Serialize, Deserialize)]
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
