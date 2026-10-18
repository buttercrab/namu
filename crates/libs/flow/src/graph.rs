use std::marker::PhantomData;

use namu_core::ValueId;
use namu_core::ir::{Call, Collect, FanOut, Literal, Next, Operation, Phi, Workflow};

use crate::ir::{BasicBlock, NodeId};
use crate::{Node, NodeKind, Terminator};

/// A handle to a value in the graph under construction. It only holds an id, so it is `Copy`
/// for every `T` (a derive would wrongly require `T: Copy`).
#[derive(Debug)]
pub struct TracedValue<T> {
    pub id: ValueId,
    _phantom: PhantomData<T>,
}

impl<T> Clone for TracedValue<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for TracedValue<T> {}

impl<T> TracedValue<T> {
    pub fn new(id: ValueId) -> Self {
        Self {
            id,
            _phantom: PhantomData,
        }
    }
}

#[derive(Default)]
pub struct NodeArena {
    pub nodes: Vec<Node>,
}

impl NodeArena {
    pub fn new_node(&mut self, kind: NodeKind, outputs: Vec<ValueId>) -> NodeId {
        let id = self.nodes.len();

        self.nodes.push(Node { kind, outputs });

        id
    }
}

#[derive(Default)]
pub struct ValueArena {
    pub next: ValueId,
}

impl ValueArena {
    pub fn new_value(&mut self) -> ValueId {
        let id = self.next;
        self.next += 1;
        id
    }

    pub fn new_values(&mut self, n: usize) -> Vec<ValueId> {
        (0..n).map(|_| self.new_value()).collect()
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
        let mut block_first_op: Vec<Option<usize>> = vec![None; self.blocks.len()];
        let mut block_last_op: Vec<Option<usize>> = vec![None; self.blocks.len()];

        // Track max ValueId encountered so we can allocate placeholders if needed
        let mut next_value_id: usize = 0;

        // First pass: convert every Node into an Operation and link linear flow within block.
        // An Operation holds literals and phis followed by at most one action (call, fan-out or
        // collect), so a new Operation starts whenever another node follows an action.
        for (block_idx, block) in self.blocks.iter().enumerate() {
            let mut prev_op_idx: Option<usize> = None;

            // Accumulated components for the *next* Operation we will emit
            let mut pending = PendingOp::default();

            for &node_id in &block.instructions {
                let node = &self.arena.nodes[node_id];

                // Update next_value_id
                if let Some(max_out) = node.outputs.iter().max() {
                    next_value_id = next_value_id.max(max_out + 1);
                }

                if pending.has_action() {
                    let op_idx = pending.flush(&mut ops);
                    link(
                        &mut ops,
                        &mut block_first_op[block_idx],
                        prev_op_idx,
                        op_idx,
                    );
                    prev_op_idx = Some(op_idx);
                }

                match &node.kind {
                    NodeKind::Literal { debug_repr, .. } => {
                        pending.literals.push(Literal {
                            output: node.outputs[0],
                            value: debug_repr.clone(),
                        });
                    }
                    NodeKind::Phi { from } => {
                        pending.phis.push(Phi {
                            output: node.outputs[0],
                            from: from.clone(),
                        });
                    }
                    NodeKind::Call { task_id, inputs } => {
                        pending.call = Some(Call {
                            task_id: task_id.clone(),
                            inputs: inputs.clone(),
                            outputs: node.outputs.clone(),
                        });
                    }
                    NodeKind::FanOut { input } => {
                        pending.fan_out = Some(FanOut {
                            input: *input,
                            output: node.outputs[0],
                        });
                    }
                    NodeKind::Collect { input } => {
                        pending.collect = Some(Collect {
                            input: *input,
                            output: node.outputs[0],
                        });
                    }
                }
            }

            // After consuming all nodes in the block, flush any remaining pending parts.
            if !pending.is_empty() {
                let op_idx = pending.flush(&mut ops);
                link(
                    &mut ops,
                    &mut block_first_op[block_idx],
                    prev_op_idx,
                    op_idx,
                );
                prev_op_idx = Some(op_idx);
            }

            // Record last op for this block
            block_last_op[block_idx] = prev_op_idx;

            // If block had no instructions, create a synthetic placeholder so that
            // control-flow edges can still target a valid op.
            if prev_op_idx.is_none() {
                let placeholder_value = next_value_id;
                next_value_id += 1;
                let op = Operation {
                    literals: vec![Literal {
                        output: placeholder_value,
                        value: "()".to_string(),
                    }],
                    phis: Vec::new(),
                    call: None,
                    fan_out: None,
                    collect: None,
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
                } => Next::Branch {
                    var: *condition,
                    true_next: block_first_op[*true_target].unwrap(),
                    false_next: block_first_op[*false_target].unwrap(),
                },
                Terminator::Return { value } => Next::Return { var: *value },
            };
            ops[last_op_idx].next = next_field;
        }

        // Third pass: patch phi source blocks (BlockId -> OpId)
        for op in &mut ops {
            for phi in &mut op.phis {
                for source in phi.from.iter_mut() {
                    let block_id = source.0;
                    let val_id = source.1;
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
                    NodeKind::Call { task_id, inputs } => {
                        let parent_vars: Vec<String> =
                            inputs.iter().map(|p| format!("var{}", p)).collect();
                        format!(
                            "  let var{} = {}({});\n",
                            node_id,
                            task_id,
                            parent_vars.join(", ")
                        )
                    }
                    NodeKind::FanOut { input } => {
                        format!("  let var{} = fan_out(var{});\n", node_id, input)
                    }
                    NodeKind::Collect { input } => {
                        format!("  let var{} = collect(var{});\n", node_id, input)
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

// --- Helper ----------------------------------------------------------------

#[derive(Default)]
struct PendingOp {
    literals: Vec<Literal>,
    phis: Vec<Phi>,
    call: Option<Call>,
    fan_out: Option<FanOut>,
    collect: Option<Collect>,
}

impl PendingOp {
    fn has_action(&self) -> bool {
        self.call.is_some() || self.fan_out.is_some() || self.collect.is_some()
    }

    fn is_empty(&self) -> bool {
        self.literals.is_empty() && self.phis.is_empty() && !self.has_action()
    }

    fn flush(&mut self, ops: &mut Vec<Operation>) -> usize {
        let taken = std::mem::take(self);
        ops.push(Operation {
            literals: taken.literals,
            phis: taken.phis,
            call: taken.call,
            fan_out: taken.fan_out,
            collect: taken.collect,
            next: Next::Return { var: None }, // placeholder
        });
        ops.len() - 1
    }
}

/// Links `op_idx` after `prev_op_idx` within a block, or records it as the block's first op.
fn link(
    ops: &mut [Operation],
    block_first_op: &mut Option<usize>,
    prev_op_idx: Option<usize>,
    op_idx: usize,
) {
    if let Some(prev_idx) = prev_op_idx {
        ops[prev_idx].next = Next::Jump { next: op_idx };
    } else {
        *block_first_op = Some(op_idx);
    }
}
