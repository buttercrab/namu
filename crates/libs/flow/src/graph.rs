use std::marker::PhantomData;

use namu_core::ValueId;
use namu_core::ir::{
    Action, Builtin, Call, Catch, Collect, FanOut, Literal, Next, Operation, Phi, Reduce, Workflow,
};
use namu_core::types::{Defs, Type};

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
    /// Each value's type, by value id.
    pub types: Vec<Option<Type>>,
    pub defs: Defs,
    _phantom: PhantomData<T>,
}

impl<T> Graph<T> {
    pub fn new(
        arena: NodeArena,
        blocks: Vec<BasicBlock>,
        types: Vec<Option<Type>>,
        defs: Defs,
    ) -> Self {
        Self {
            arena,
            blocks,
            types,
            defs,
            _phantom: PhantomData,
        }
    }

    pub fn to_serializable(&self, name: String) -> Workflow {
        // Mapping helpers
        let mut ops: Vec<Operation> = Vec::new();
        let mut block_first_op: Vec<Option<usize>> = vec![None; self.blocks.len()];
        let mut block_last_op: Vec<Option<usize>> = vec![None; self.blocks.len()];
        // Constants depend on nothing, so they are evaluated once, in the root context, where
        // every branch and every scope can see them (see "level 2 dedup" in docs/design.md).
        let mut constants: Vec<Literal> = Vec::new();

        // First pass: convert every Node into an Operation and link linear flow within block.
        // An Operation holds phis followed by at most one action (call, fan-out or
        // collect), so a new Operation starts whenever another node follows an action.
        for (block_idx, block) in self.blocks.iter().enumerate() {
            let mut prev_op_idx: Option<usize> = None;

            // Accumulated components for the *next* Operation we will emit
            let mut pending = PendingOp::default();

            for &node_id in &block.instructions {
                let node = &self.arena.nodes[node_id];

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
                    NodeKind::Literal { json } => {
                        constants.push(Literal {
                            output: node.outputs[0],
                            value: json.clone(),
                        });
                    }
                    NodeKind::Phi { from } => {
                        pending.phis.push(Phi {
                            output: node.outputs[0],
                            from: from.clone(),
                        });
                    }
                    NodeKind::Call { task_id, inputs } => {
                        pending.action = Some(Action::Call(Call {
                            task_id: task_id.clone(),
                            inputs: inputs.clone(),
                            outputs: node.outputs.clone(),
                            catch: None,
                        }));
                    }
                    NodeKind::FanOut { input } => {
                        pending.action = Some(Action::FanOut(FanOut {
                            input: *input,
                            output: node.outputs[0],
                        }));
                    }
                    NodeKind::Collect { input, reduce } => {
                        pending.action = Some(Action::Collect(Collect {
                            input: *input,
                            output: node.outputs[0],
                            reduce: reduce.as_ref().map(|(task_id, init)| Reduce {
                                task_id: task_id.clone(),
                                init: *init,
                            }),
                        }));
                    }
                    NodeKind::Builtin { op, ty, inputs } => {
                        pending.builtins.push(Builtin {
                            op: *op,
                            ty: *ty,
                            inputs: inputs.clone(),
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

            // An empty block still needs an operation for control flow to target; its
            // terminator is patched in below.
            if prev_op_idx.is_none() {
                ops.push(Operation::new(
                    Vec::new(),
                    Vec::new(),
                    None,
                    Next::return_unit(),
                ));
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
                Terminator::Catch { ok, err, error } => {
                    let Some(Action::Call(call)) = &mut ops[last_op_idx].action else {
                        panic!("block {block_idx} catches, but does not end with a call");
                    };
                    call.catch = Some(Catch {
                        error: *error,
                        next: block_first_op[*err].unwrap(),
                    });
                    Next::Jump {
                        next: block_first_op[*ok].unwrap(),
                    }
                }
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

        // Block 0 is the entry, and its first operation is op 0.
        ops[0].literals = constants;

        // Every value the builder made has a type; one without is a bug in the builder.
        let types = self
            .types
            .iter()
            .enumerate()
            .map(|(value, ty)| {
                ty.clone()
                    .unwrap_or_else(|| panic!("value {value} of workflow {name} has no type"))
            })
            .collect();
        Workflow {
            name,
            operations: ops,
            types,
            defs: self.defs.clone(),
        }
    }

    pub fn graph_string(&self) -> String {
        let mut s = String::new();
        for (i, block) in self.blocks.iter().enumerate() {
            s.push_str(&format!("Block {}:\n", i));
            for &node_id in &block.instructions {
                let node = &self.arena.nodes[node_id];
                let line = match &node.kind {
                    NodeKind::Literal { json } => {
                        format!("  let var{} = {};\n", node_id, json)
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
                    NodeKind::Collect {
                        input,
                        reduce: None,
                    } => format!("  let var{} = collect(var{});\n", node_id, input),
                    NodeKind::Collect {
                        input,
                        reduce: Some((task, init)),
                    } => format!("  let var{node_id} = reduce({task}, var{init}, var{input});\n"),
                    NodeKind::Builtin { op, ty, inputs } => {
                        let inputs: Vec<String> =
                            inputs.iter().map(|v| format!("var{v}")).collect();
                        format!(
                            "  let var{} = {op:?}::<{ty:?}>({});\n",
                            node_id,
                            inputs.join(", ")
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
                    Terminator::Catch { ok, err, error } => {
                        format!("  ok -> Block {}, err var{} -> Block {}", ok, error, err)
                    }
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
    phis: Vec<Phi>,
    builtins: Vec<Builtin>,
    action: Option<Action>,
}

impl PendingOp {
    fn has_action(&self) -> bool {
        self.action.is_some()
    }

    fn is_empty(&self) -> bool {
        self.phis.is_empty() && self.builtins.is_empty() && !self.has_action()
    }

    fn flush(&mut self, ops: &mut Vec<Operation>) -> usize {
        let taken = std::mem::take(self);
        // The terminator is linked in later.
        ops.push(
            Operation::new(Vec::new(), taken.phis, taken.action, Next::return_unit())
                .with_builtins(taken.builtins),
        );
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
