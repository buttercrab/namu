use std::any::Any;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt::Debug;
use std::marker::PhantomData;
use std::sync::Arc;

pub use macros::{task, workflow};

// --- Core Data Structures ---

pub type Value = Arc<dyn Any + Send + Sync>;
pub type Executable = Arc<dyn Fn(Vec<Value>) -> Value + Send + Sync>;

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
        func: Executable,
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

    pub fn branch(condition: NodeId, true_target: BlockId, false_target: BlockId) -> Self {
        Self::Branch {
            condition,
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

// --- Graph API ---

impl<T> Graph<T> {
    pub fn new(arena: Arena, blocks: Vec<BasicBlock>) -> Self {
        Self {
            arena,
            blocks,
            _phantom: PhantomData,
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

impl<T: Clone + 'static> Graph<T> {
    pub fn run(&self) -> T {
        let mut results = HashMap::<NodeId, Value>::new();
        let mut queue: VecDeque<(BlockId, BlockId)> = VecDeque::new(); // (current_block_id, prev_block_id)

        if !self.blocks.is_empty() {
            queue.push_back((0, 0)); // Start at block 0, prev is dummy 0
        }

        let mut executed_blocks = HashSet::new();

        while let Some((block_id, prev_block_id)) = queue.pop_front() {
            if executed_blocks.contains(&block_id) {
                continue;
            }
            executed_blocks.insert(block_id);

            let block = &self.blocks[block_id];

            // 1. Execute Phi nodes
            for &node_id in &block.instructions {
                if let NodeKind::Phi { from } = &self.arena.nodes[node_id].kind {
                    let (_, value_node_id) = from
                        .iter()
                        .find(|(from_block_id, _)| *from_block_id == prev_block_id)
                        .expect(
                            "Invalid CFG: Phi node does not have an entry for predecessor block.",
                        );

                    if let Some(value) = results.get(value_node_id) {
                        results.insert(node_id, value.clone());
                    }
                }
            }

            // 2. Execute other instructions
            for &node_id in &block.instructions {
                if results.contains_key(&node_id) {
                    continue; // Skip phis
                }
                let node = &self.arena.nodes[node_id];
                let value = match &node.kind {
                    NodeKind::Literal { value, .. } => value.clone(),
                    NodeKind::Call { func, inputs, .. } => {
                        let input_values = inputs
                            .iter()
                            .map(|&input_id| results[&input_id].clone())
                            .collect::<Vec<_>>();
                        func(input_values)
                    }
                    NodeKind::Phi { .. } => unreachable!(), // Handled above
                };
                results.insert(node_id, value);
            }

            // 3. Follow terminator
            if let Some(terminator) = &block.terminator {
                match terminator {
                    Terminator::Jump { target } => {
                        queue.push_back((*target, block_id));
                    }
                    Terminator::Branch {
                        condition,
                        true_target,
                        false_target,
                    } => {
                        let cond_value = results[condition].downcast_ref::<bool>().unwrap();
                        if *cond_value {
                            queue.push_back((*true_target, block_id));
                        } else {
                            queue.push_back((*false_target, block_id));
                        }
                    }
                    Terminator::Return { value: Some(value) } => {
                        let final_value = results.get(value).unwrap();
                        return final_value.downcast_ref::<T>().unwrap().clone();
                    }
                    Terminator::Return { value: None } => {
                        unimplemented!()
                    }
                }
            }
        }

        // Handle case for empty graph returning unit `()`
        if self.blocks.is_empty() {
            let unit_val: Value = Arc::new(());
            if let Some(ret) = unit_val.downcast_ref::<T>() {
                return ret.clone();
            }
        }

        panic!("Workflow did not return a value");
    }
}

// --- Node Creation ---

pub fn new_literal<T: Debug + Send + Sync + 'static, U>(
    builder: &mut Builder<U>,
    value: T,
) -> TracedValue<T> {
    let debug_repr = format!("{:?}", value);
    builder.add_instruction(NodeKind::Literal {
        value: Arc::new(value),
        debug_repr,
    })
}

pub fn phi<G: 'static, T: Clone + 'static>(
    builder: &mut Builder<G>,
    from: Vec<(BlockId, TracedValue<T>)>,
) -> TracedValue<T> {
    if from.is_empty() {
        panic!("phi node must have at least one incoming value");
    }

    let first_node_id = from[0].1.id;
    if from
        .iter()
        .skip(1)
        .all(|(_, value)| value.id == first_node_id)
    {
        return TracedValue::new(first_node_id);
    }

    let from = from
        .into_iter()
        .map(|(block, value)| (block, value.id))
        .collect();

    builder.add_instruction(NodeKind::Phi { from })
}

pub struct Builder<T> {
    pub arena: Arena,
    pub blocks: Vec<BasicBlock>,
    pub current_block_id: BlockId,
    _phantom: PhantomData<T>,
}

impl<T> Default for Builder<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Builder<T> {
    pub fn new() -> Self {
        Self {
            arena: Arena::default(),
            blocks: vec![BasicBlock::default()],
            current_block_id: 0,
            _phantom: PhantomData,
        }
    }

    pub fn add_instruction<V>(&mut self, kind: NodeKind) -> TracedValue<V> {
        let id = self.arena.new_node(kind);
        self.blocks[self.current_block_id].instructions.push(id);
        TracedValue::new(id)
    }

    pub fn seal_block(&mut self, terminator: Terminator) {
        self.blocks[self.current_block_id].terminator = Some(terminator);
    }

    pub fn new_block(&mut self) -> BlockId {
        let block_id = self.blocks.len();
        self.blocks.push(BasicBlock::default());
        block_id
    }

    pub fn switch_to_block(&mut self, block_id: BlockId) {
        self.current_block_id = block_id;
    }

    pub fn build(self) -> Graph<T> {
        Graph::new(self.arena, self.blocks)
    }
}
