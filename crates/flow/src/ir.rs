//! The Intermediate Representation (IR) of the computational graph.
//!
//! This module defines the core data structures that represent the graph,
//! such as `Graph`, `Node`, `BasicBlock`, and `Terminator`.

use std::any::Any;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::Arc;

use namu_core::ir::{Next, OpKind, Operation, Workflow};

// --- Core Data Structures ---

pub type Value = Arc<dyn Any + Send + Sync>;
pub type NodeId = usize;
pub type BlockId = usize;

#[derive(Default)]
pub struct NodeArena {
    pub nodes: Vec<Node>,
}

impl NodeArena {
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
        task_name: &'static str,
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
    Extract {
        tuple: NodeId,
        index: usize,
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
    pub arena: NodeArena,
    pub blocks: Vec<BasicBlock>,
    _phantom: PhantomData<T>,
}

impl<T> Graph<T> {
    pub fn new(arena: NodeArena, blocks: Vec<BasicBlock>) -> Self {
        Self {
            arena,
            blocks,
            _phantom: PhantomData,
        }
    }

    pub fn to_serializable(&self, name: String) -> Workflow {
        // Mapping helpers
        let mut ops: Vec<Operation> = Vec::new();
        let mut node_to_value: HashMap<NodeId, usize> = HashMap::new();
        let mut block_first_op: Vec<Option<usize>> = vec![None; self.blocks.len()];
        let mut block_last_op: Vec<Option<usize>> = vec![None; self.blocks.len()];

        // Unique value ids – reuse node ids for now (they are unique)
        let mut next_value_id: usize = 0;

        // First pass: convert every Node into an Operation and link linear flow within block.
        for (block_idx, block) in self.blocks.iter().enumerate() {
            let mut prev_op_idx: Option<usize> = None;

            for &node_id in &block.instructions {
                let value_id = next_value_id;
                next_value_id += 1;
                node_to_value.insert(node_id, value_id);

                let (kind, outputs) = match &self.arena.nodes[node_id].kind {
                    NodeKind::Literal { debug_repr, .. } => (
                        OpKind::Literal {
                            value: debug_repr.clone(),
                        },
                        vec![value_id],
                    ),
                    NodeKind::Phi { from } => {
                        // temporarily store BlockId; patch later
                        let from_placeholder: Vec<(usize, usize)> =
                            from.iter().map(|(b, v)| (*b as usize, *v)).collect();
                        (
                            OpKind::Phi {
                                from: from_placeholder,
                            },
                            vec![value_id],
                        )
                    }
                    NodeKind::Call {
                        task_id, inputs, ..
                    } => {
                        let input_values: Vec<usize> =
                            inputs.iter().map(|n| node_to_value[n]).collect();
                        (
                            OpKind::Call {
                                name: task_id.clone(),
                                inputs: input_values,
                            },
                            vec![value_id],
                        )
                    }
                    NodeKind::Extract { tuple, index } => {
                        let tuple_val = node_to_value[tuple];
                        (
                            OpKind::Extract {
                                tuple: tuple_val,
                                index: *index,
                            },
                            vec![value_id],
                        )
                    }
                };

                let op = Operation {
                    kind,
                    outputs,
                    next: Next::Return { var: None }, // placeholder; will patch
                };
                ops.push(op);
                let current_op_idx = ops.len() - 1;

                // Link previous op in this block to current
                if let Some(prev_idx) = prev_op_idx {
                    ops[prev_idx].next = Next::Jump {
                        next: current_op_idx,
                    };
                } else {
                    block_first_op[block_idx] = Some(current_op_idx);
                }

                prev_op_idx = Some(current_op_idx);
            }

            // Record last op for this block
            block_last_op[block_idx] = prev_op_idx;

            // If block had no instructions, create a synthetic placeholder so that
            // control-flow edges can still target a valid op.
            if prev_op_idx.is_none() {
                let placeholder_value = next_value_id;
                next_value_id += 1;
                let op = Operation {
                    kind: OpKind::Literal {
                        value: "()".to_string(),
                    },
                    outputs: vec![placeholder_value],
                    next: Next::Return { var: None },
                };
                ops.push(op);
                block_first_op[block_idx] = Some(ops.len() - 1);
                block_last_op[block_idx] = Some(ops.len() - 1);
            }
        }

        // Second pass: patch block terminators & phi sources.
        for (block_idx, block) in self.blocks.iter().enumerate() {
            let last_op_idx = block_last_op[block_idx].expect("Block without operations");

            // Terminator patching
            let next_field = match block.terminator.as_ref().expect("Block not sealed") {
                Terminator::Jump { target } => Next::Jump {
                    next: block_first_op[*target].unwrap(),
                },
                Terminator::Branch {
                    condition,
                    true_target,
                    false_target,
                } => {
                    let cond_value = node_to_value[condition];
                    Next::Branch {
                        var: cond_value,
                        true_next: block_first_op[*true_target].unwrap(),
                        false_next: block_first_op[*false_target].unwrap(),
                    }
                }
                Terminator::Return { value } => {
                    let var = value.map(|nid| node_to_value[&nid]);
                    Next::Return { var }
                }
            };
            ops[last_op_idx].next = next_field;
        }

        // Third pass: patch phi source blocks (BlockId -> OpId)
        for op_idx in 0..ops.len() {
            if let OpKind::Phi { from } = &mut ops[op_idx].kind {
                for source in from.iter_mut() {
                    let block_id = source.0;
                    let node_id = source.1;
                    let val_id = node_to_value[&node_id];
                    *source = (block_last_op[block_id].unwrap(), val_id);
                }
            }
        }

        Workflow {
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
                    NodeKind::Call {
                        task_name: name,
                        inputs,
                        ..
                    } => {
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
                    NodeKind::Extract { tuple, index } => {
                        format!("  let var{} = extract(var{}, {});\n", node_id, tuple, index)
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
