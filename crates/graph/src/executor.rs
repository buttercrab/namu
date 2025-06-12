//! The graph execution engine.
//!
//! This module contains the `Executor` responsible for running a computational
//! graph, as well as the `TaskRegistry` for managing task implementations.

use std::any::Any;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use once_cell::sync::Lazy;

use crate::ir::{BlockId, Graph, NodeId, NodeKind, Value};

// --- Task Registry ---

pub type Executable = Arc<dyn Fn(Vec<Value>) -> Value + Send + Sync>;
pub type TaskFactory = Arc<dyn Fn() -> Executable + Send + Sync>;

// While we move to an explicit registry, we keep the global one for now
// to support the existing `#[task]` macro registration mechanism.
static TASK_REGISTRY: Lazy<Mutex<HashMap<String, TaskFactory>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

pub fn register_task(task_id: String, factory: TaskFactory) {
    TASK_REGISTRY.lock().unwrap().insert(task_id, factory);
}

// --- Executor ---

pub struct Executor;

impl Executor {
    pub fn new() -> Self {
        Self
    }

    pub fn run<T: Clone + 'static>(&self, graph: &Graph<T>) -> T {
        let mut results = HashMap::<NodeId, Value>::new();
        let mut queue: VecDeque<(BlockId, BlockId)> = VecDeque::new(); // (current_block_id, prev_block_id)
        let mut return_value = None;

        if !graph.blocks.is_empty() {
            queue.push_back((0, 0)); // Start at block 0, prev is dummy 0
        }

        while let Some((block_id, prev_block_id)) = queue.pop_front() {
            let block = &graph.blocks[block_id];

            let mut phi_values = HashMap::new();

            // First pass: resolve all phi nodes in the block.
            // This is crucial because phi nodes in a block must be resolved simultaneously,
            // using only values from the predecessor block.
            for &node_id in &block.instructions {
                if let NodeKind::Phi { from } = &graph.arena.nodes[node_id].kind {
                    let (_, value_node_id) = from
                        .iter()
                        .find(|(from_block_id, _)| *from_block_id == prev_block_id)
                        .unwrap_or_else(|| {
                            panic!(
                                "Invalid CFG: Phi node {} in block {} does not have an entry for predecessor block {}.",
                                node_id, block_id, prev_block_id
                            )
                        });

                    let value = results
                        .get(value_node_id)
                        .unwrap_or_else(|| {
                            panic!(
                                "Value for node {} (source for phi {} in block {}) not found.",
                                value_node_id, node_id, block_id
                            )
                        })
                        .clone();
                    phi_values.insert(node_id, value);
                }
            }
            results.extend(phi_values);

            // Second pass: execute all other instructions.
            for &node_id in &block.instructions {
                if matches!(graph.arena.nodes[node_id].kind, NodeKind::Phi { .. }) {
                    continue; // Already processed
                }

                let node = &graph.arena.nodes[node_id];
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
                    NodeKind::Phi { .. } => {
                        // This is handled in the first pass
                        unreachable!()
                    }
                };
                results.insert(node_id, value);
            }

            if let Some(terminator) = &block.terminator {
                match terminator {
                    crate::ir::Terminator::Jump { target } => {
                        queue.push_back((*target, block_id));
                    }
                    crate::ir::Terminator::Branch {
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
                    crate::ir::Terminator::Return { value } => {
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

impl Default for Executor {
    fn default() -> Self {
        Self::new()
    }
}
