use once_cell::sync::Lazy;
use std::any::Any;
use std::cell::{Ref, RefCell, RefMut};
use std::collections::{HashMap, VecDeque};
use std::fmt::Debug;
use std::marker::PhantomData;
use std::sync::{Arc, Mutex};

pub use macros::{task, workflow};

// --- Core Data Structures ---

pub type Value = Arc<dyn Any + Send + Sync>;
pub type Executable = Arc<dyn Fn(Vec<Value>) -> Value + Send + Sync>;
pub type TaskFactory = Arc<dyn Fn() -> Executable + Send + Sync>;
static TASK_REGISTRY: Lazy<Mutex<HashMap<String, TaskFactory>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

pub fn register_task(task_id: String, factory: TaskFactory) {
    TASK_REGISTRY.lock().unwrap().insert(task_id, factory);
}

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

struct BuilderInner {
    arena: Arena,
    blocks: Vec<BasicBlock>,
    current_block_id: BlockId,
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
        let mut return_value = None;

        if !self.blocks.is_empty() {
            queue.push_back((0, 0)); // Start at block 0, prev is dummy 0
        }

        while let Some((block_id, prev_block_id)) = queue.pop_front() {
            let block = &self.blocks[block_id];

            for &node_id in &block.instructions {
                let node = &self.arena.nodes[node_id];
                let value = match &node.kind {
                    NodeKind::Literal { value, .. } => value.clone(),
                    NodeKind::Call {
                        task_id, inputs, ..
                    } => {
                        let registry = TASK_REGISTRY.lock().unwrap();
                        let factory = registry
                            .get(task_id)
                            .unwrap_or_else(|| panic!("Task not found in registry: {}", task_id));
                        let func = factory(); // Create the Executable on-the-fly
                        let input_values = inputs
                            .iter()
                            .map(|&input_id| results[&input_id].clone())
                            .collect::<Vec<_>>();
                        func(input_values)
                    }
                    NodeKind::Phi { from } => {
                        let (_, value_node_id) = from
                        .iter()
                        .find(|(from_block_id, _)| *from_block_id == prev_block_id)
                        .expect(
                            "Invalid CFG: Phi node does not have an entry for predecessor block.",
                        );

                        results
                            .get(value_node_id)
                            .expect("Phi node has no value")
                            .clone()
                    }
                };
                results.insert(node_id, value);
            }

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
                        let cond_val = results[condition].downcast_ref::<bool>().unwrap();
                        if *cond_val {
                            queue.push_back((*true_target, block_id));
                        } else {
                            queue.push_back((*false_target, block_id));
                        }
                    }
                    Terminator::Return { value } => {
                        return_value = *value;
                        break;
                    }
                }
            }
        }

        if let Some(return_node_id) = return_value {
            results
                .get(&return_node_id)
                .and_then(|v| v.downcast_ref::<T>().cloned())
                .expect("Workflow returned a value of the wrong type")
        } else if std::any::TypeId::of::<T>() == std::any::TypeId::of::<()>() {
            // If the expected return type is unit, we can fake it.
            // This is a bit of a hack, but it's necessary to support workflows that
            // don't explicitly return a value.
            let unit: Arc<dyn Any + Send + Sync> = Arc::new(());
            unit.downcast_ref::<T>().cloned().unwrap()
        } else {
            panic!("Workflow did not return a value");
        }
    }
}

// --- Builder API ---

pub fn new_literal<T: Debug + Send + Sync + 'static, U>(
    builder: &Builder<U>,
    value: T,
) -> TracedValue<T> {
    let debug_repr = format!("{:?}", value);
    let kind = NodeKind::Literal {
        value: Arc::new(value),
        debug_repr,
    };
    builder.add_instruction(kind)
}

pub fn phi<G: 'static, T: Clone + 'static>(
    builder: &Builder<G>,
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
    inner: RefCell<BuilderInner>,
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
            inner: RefCell::new(BuilderInner {
                arena: Arena::default(),
                blocks: vec![BasicBlock::default()],
                current_block_id: 0,
            }),
            _phantom: PhantomData,
        }
    }

    pub fn current_block_id(&self) -> BlockId {
        self.inner.borrow().current_block_id
    }

    pub fn arena(&self) -> Ref<Arena> {
        Ref::map(self.inner.borrow(), |inner| &inner.arena)
    }

    pub fn arena_mut(&self) -> RefMut<Arena> {
        RefMut::map(self.inner.borrow_mut(), |inner| &mut inner.arena)
    }

    pub fn add_instruction<V>(&self, kind: NodeKind) -> TracedValue<V> {
        let mut inner = self.inner.borrow_mut();
        let id = inner.arena.new_node(kind);
        let current_block_id = inner.current_block_id;
        inner.blocks[current_block_id].instructions.push(id);
        TracedValue::new(id)
    }

    pub fn add_instruction_to_current_block(&self, node_id: NodeId) {
        let mut inner = self.inner.borrow_mut();
        let current_block_id = inner.current_block_id;
        inner.blocks[current_block_id].instructions.push(node_id);
    }

    pub fn seal_block(&self, terminator: Terminator) {
        let mut inner = self.inner.borrow_mut();
        let current_block_id = inner.current_block_id;
        let current_block = &mut inner.blocks[current_block_id];
        assert!(current_block.terminator.is_none(), "Block already sealed");
        current_block.terminator = Some(terminator);
    }

    pub fn new_block(&self) -> BlockId {
        let mut inner = self.inner.borrow_mut();
        let id = inner.blocks.len();
        inner.blocks.push(BasicBlock::default());
        id
    }

    pub fn switch_to_block(&self, block_id: BlockId) {
        let mut inner = self.inner.borrow_mut();
        inner.current_block_id = block_id;
    }

    pub fn build(self) -> Graph<T> {
        let inner = self.inner.into_inner();
        Graph::new(inner.arena, inner.blocks)
    }
}
