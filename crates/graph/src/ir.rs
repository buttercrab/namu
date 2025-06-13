//! The Intermediate Representation (IR) of the computational graph.
//!
//! This module defines the core data structures that represent the graph,
//! such as `Graph`, `Node`, `BasicBlock`, and `Terminator`.

use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::Arc;
use std::{any::Any, mem};

use common::{
    Literal, Next, Operation, Phi as SerializablePhi, Task, Workflow as SerializableWorkflow,
};

// --- Core Data Structures ---

pub type Value = Arc<dyn Any + Send + Sync>;
pub type NodeId = usize;
pub type BlockId = usize;

#[derive(Default)]
pub struct Arena {
    pub nodes: Vec<Node>,
}

impl Arena {
    pub fn new_node(&mut self, kind: NodeKind) -> NodeId {
        let id = self.nodes.len();

        let inputs = match &kind {
            NodeKind::Call { inputs, .. } => inputs.clone(),
            _ => Vec::new(),
        };

        self.nodes.push(Node {
            kind,
            outputs: Vec::new(),
        });

        for &input_id in &inputs {
            self.nodes[input_id].outputs.push(id);
        }

        id
    }
}

pub enum NodeKind {
    Call {
        name: &'static str,
        task_id: String,
        inputs: Vec<NodeId>,
    },
    Literal {
        value: Value,
        debug_repr: String,
    },
    Phi {
        from: Vec<(BlockId, NodeId)>,
    },
}

pub struct Node {
    pub kind: NodeKind,
    pub outputs: Vec<NodeId>,
}

pub enum Terminator {
    Jump {
        target: BlockId,
    },
    Branch {
        condition: NodeId,
        true_target: BlockId,
        false_target: BlockId,
    },
    Return {
        value: Option<NodeId>,
    },
}

impl Terminator {
    pub fn jump(target: BlockId) -> Self {
        Self::Jump { target }
    }

    pub fn branch(
        condition: TracedValue<bool>,
        true_target: BlockId,
        false_target: BlockId,
    ) -> Self {
        Self::Branch {
            condition: condition.id,
            true_target,
            false_target,
        }
    }

    pub fn return_value(value: NodeId) -> Self {
        Self::Return { value: Some(value) }
    }

    pub fn return_unit() -> Self {
        Self::Return { value: None }
    }
}

#[derive(Default)]
pub struct BasicBlock {
    pub instructions: Vec<NodeId>,
    pub terminator: Option<Terminator>,
}

#[derive(Copy, Clone)]
pub struct TracedValue<T> {
    pub id: NodeId,
    _phantom: PhantomData<T>,
}

impl<T> TracedValue<T> {
    pub fn new(id: NodeId) -> Self {
        Self {
            id,
            _phantom: PhantomData,
        }
    }
}

pub struct Graph<T> {
    pub arena: Arena,
    pub blocks: Vec<BasicBlock>,
    _phantom: PhantomData<T>,
}

impl<T> Graph<T> {
    pub fn new(arena: Arena, blocks: Vec<BasicBlock>) -> Self {
        Self {
            arena,
            blocks,
            _phantom: PhantomData,
        }
    }

    pub fn to_serializable(&self, name: String) -> SerializableWorkflow {
        let mut ops = Vec::new();
        let mut block_id_to_op_id = HashMap::new();

        // Pass 1: Create one operation per block.
        for (block_id, block) in self.blocks.iter().enumerate() {
            block_id_to_op_id.insert(block_id, (ops.len(), ops.len()));

            let mut ops_count = 0;
            let mut phis = Vec::new();
            let mut literals = Vec::new();

            for &node_id in &block.instructions {
                let node = &self.arena.nodes[node_id];
                match &node.kind {
                    NodeKind::Phi { from } => {
                        phis.push(SerializablePhi {
                            id: node_id,
                            from: from.clone(),
                        });
                    }
                    NodeKind::Literal { debug_repr, .. } => {
                        literals.push(Literal {
                            value: debug_repr.clone(),
                            output: node_id,
                        });
                    }
                    NodeKind::Call {
                        task_id, inputs, ..
                    } => {
                        let task = Some(Task {
                            name: task_id.clone(),
                            inputs: inputs.clone(),
                            output: node_id,
                        });

                        ops.push(Operation {
                            phis: mem::take(&mut phis),
                            literals: mem::take(&mut literals),
                            task,
                            next: Next::Jump { next: ops.len() },
                        });

                        ops_count += 1;
                    }
                }
            }

            if ops_count == 0 || !phis.is_empty() || !literals.is_empty() {
                ops.push(Operation {
                    phis: mem::take(&mut phis),
                    literals: mem::take(&mut literals),
                    task: None,
                    next: Next::Return { var: None }, // Placeholder
                });
            }

            block_id_to_op_id.entry(block_id).and_modify(|(_, end)| {
                *end = ops.len() - 1;
            });
        }

        // Pass 2: Link terminators and resolve phi nodes.
        for (block_id, block) in self.blocks.iter().enumerate() {
            // Link the terminator.
            let terminator = block.terminator.as_ref().unwrap();
            let (start, end) = block_id_to_op_id[&block_id];
            ops[end].next = match terminator {
                Terminator::Jump { target } => Next::Jump {
                    next: block_id_to_op_id[target].0,
                },
                Terminator::Branch {
                    condition,
                    true_target,
                    false_target,
                } => Next::Branch {
                    var: *condition,
                    true_next: block_id_to_op_id[true_target].0,
                    false_next: block_id_to_op_id[false_target].0,
                },
                Terminator::Return { value } => Next::Return { var: *value },
            };

            // Resolve phi node sources.
            for phi in &mut ops[start].phis {
                phi.from = phi
                    .from
                    .iter()
                    .map(|(from_b, v)| (block_id_to_op_id[from_b].1, *v))
                    .collect();
            }
        }

        SerializableWorkflow {
            name,
            operations: ops,
        }
    }

    pub fn graph_string(&self) -> String {
        let mut s = String::new();
        for (i, block) in self.blocks.iter().enumerate() {
            s.push_str(&format!("Block {}:\n", i));
            for &node_id in &block.instructions {
                let node = &self.arena.nodes[node_id];
                let line = match &node.kind {
                    NodeKind::Literal { debug_repr, .. } => {
                        format!("  let var{} = {};\n", node_id, debug_repr)
                    }
                    NodeKind::Call { name, inputs, .. } => {
                        let parent_vars: Vec<String> =
                            inputs.iter().map(|p| format!("var{}", p)).collect();
                        format!(
                            "  let var{} = {}({});\n",
                            node_id,
                            name,
                            parent_vars.join(", ")
                        )
                    }
                    NodeKind::Phi { from } => {
                        let from_str: Vec<String> = from
                            .iter()
                            .map(|(b, v)| format!("[block {}, var{}]", b, v))
                            .collect();
                        format!("  let var{} = phi({});\n", node_id, from_str.join(", "))
                    }
                };
                s.push_str(&line);
            }

            if let Some(terminator) = &block.terminator {
                let term_str = match terminator {
                    Terminator::Jump { target } => format!("  jump -> Block {}", target),
                    Terminator::Branch {
                        condition,
                        true_target,
                        false_target,
                    } => format!(
                        "  branch var{} ? Block {} : Block {}",
                        condition, true_target, false_target
                    ),
                    Terminator::Return { value } => {
                        if let Some(value) = value {
                            format!("  return var{}", value)
                        } else {
                            "  return ()".to_string()
                        }
                    }
                };
                s.push_str(&term_str);
                s.push('\n');
            }
        }
        s
    }
}
