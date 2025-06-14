use std::{
    any::Any,
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use common::VarId;
use scc::{HashIndex, HashMap as SccHashMap};

use crate::context::ContextManager;

#[derive(Clone, Debug)]
struct ContextTreeNode {
    ancestors: Vec<usize>,
    depth: usize,
    order: usize,
    var_id: VarId,
    value: Arc<dyn Any + Send + Sync>,
}

pub struct SimpleContextManager {
    nodes: HashIndex<usize, ContextTreeNode>,
    node_state: SccHashMap<usize, (bool, usize)>, // (is_used, children_count)
    id_counter: AtomicUsize,
}

impl SimpleContextManager {
    pub fn new() -> Self {
        Self {
            nodes: HashIndex::new(),
            node_state: SccHashMap::new(),
            id_counter: AtomicUsize::new(0),
        }
    }

    fn new_node(
        &self,
        parent: Option<usize>,
        var_id: VarId,
        value: Arc<dyn Any + Send + Sync>,
    ) -> usize {
        let order = parent.map_or(0, |parent_id| {
            self.node_state
                .update(&parent_id, |_, (_, children)| *children + 1)
                .unwrap()
        });

        let (depth, ancestors) = if let Some(parent_id) = parent {
            let parent_node = self.nodes.get(&parent_id).unwrap();
            let mut ancestors = vec![parent_id];
            let mut i = 0;
            while let Some(p_ancestor) = self
                .nodes
                .get(ancestors.last().unwrap())
                .unwrap()
                .ancestors
                .get(i)
            {
                ancestors.push(*p_ancestor);
                i += 1;
            }
            (parent_node.depth + 1, ancestors)
        } else {
            (0, vec![])
        };

        let node = ContextTreeNode {
            ancestors,
            depth,
            order,
            var_id,
            value,
        };
        let id = self.id_counter.fetch_add(1, Ordering::Relaxed);
        self.nodes.insert(id, node).unwrap();
        self.node_state.insert(id, (true, 0)).unwrap();
        id
    }
}

impl ContextManager for SimpleContextManager {
    type ContextId = usize;

    fn create_context(&self) -> Self::ContextId {
        let id = self.new_node(None, 0, Arc::new(()));
        id
    }

    fn compare_context(&self, a: Self::ContextId, b: Self::ContextId) -> std::cmp::Ordering {
        if a == b {
            return std::cmp::Ordering::Equal;
        }

        let mut node_a = self.nodes.get(&a).unwrap();
        let mut node_b = self.nodes.get(&b).unwrap();
        let mut current_a = a;
        let mut current_b = b;

        // Equalize depths using binary lifting
        if node_a.depth > node_b.depth {
            let diff = node_a.depth - node_b.depth;
            for i in (0..node_a.ancestors.len()).rev() {
                if (diff >> i) & 1 == 1 {
                    current_a = node_a.ancestors[i];
                    node_a = self.nodes.get(&current_a).unwrap();
                }
            }
        } else if node_b.depth > node_a.depth {
            let diff = node_b.depth - node_a.depth;
            for i in (0..node_b.ancestors.len()).rev() {
                if (diff >> i) & 1 == 1 {
                    current_b = node_b.ancestors[i];
                    node_b = self.nodes.get(&current_b).unwrap();
                }
            }
        }

        if current_a == current_b {
            // One is an ancestor of the other. The original `a` or `b` with greater depth is "larger".
            return a.cmp(&b).reverse();
        }

        // Move up together to find children of LCA
        for i in (0..node_a.ancestors.len()).rev() {
            let ancestor_a = node_a.ancestors.get(i);
            let ancestor_b = node_b.ancestors.get(i);
            if ancestor_a != ancestor_b {
                current_a = *ancestor_a.unwrap();
                current_b = *ancestor_b.unwrap();
                node_a = self.nodes.get(&current_a).unwrap();
                node_b = self.nodes.get(&current_b).unwrap();
            }
        }

        // Now current_a and current_b are the children of the LCA
        node_a.order.cmp(&node_b.order)
    }

    fn add_variable(
        &self,
        context_id: Self::ContextId,
        var_id: VarId,
        value: Arc<dyn Any + Send + Sync>,
    ) -> Self::ContextId {
        self.new_node(Some(context_id), var_id, value)
    }

    fn get_variable(
        &self,
        context_id: Self::ContextId,
        var_id: VarId,
    ) -> Option<Arc<dyn Any + Send + Sync>> {
        let mut current_id = context_id;
        loop {
            let node = self.nodes.get(&current_id).unwrap();
            if node.var_id == var_id {
                return Some(node.value.clone());
            }
            if let Some(parent_id) = node.ancestors.first() {
                current_id = *parent_id;
            } else {
                return None;
            }
        }
    }

    fn get_variables(
        &self,
        context_id: Self::ContextId,
        var_ids: &[VarId],
    ) -> Vec<Arc<dyn Any + Send + Sync>> {
        if var_ids.is_empty() {
            return vec![];
        }

        let mut found_vars = HashMap::with_capacity(var_ids.len());
        let mut to_find: HashSet<VarId> = var_ids.iter().copied().collect();

        if !to_find.is_empty() {
            let mut current_id = Some(context_id);
            while let Some(id) = current_id {
                if to_find.is_empty() {
                    break;
                }

                let node = self.nodes.get(&id).unwrap();

                if to_find.remove(&node.var_id) {
                    found_vars.insert(node.var_id, node.value.clone());
                }

                current_id = node.ancestors.first().copied();
            }
        }

        var_ids
            .iter()
            .map(|var_id| found_vars.get(var_id).unwrap().clone())
            .collect()
    }

    fn remove_context(&self, mut context_id: Self::ContextId) {
        if self
            .node_state
            .update(&context_id, |_, (is_used, _)| *is_used = false)
            .is_none()
        {
            return;
        }

        loop {
            let parent_id = if let Some(node) = self.nodes.get(&context_id) {
                node.ancestors.first().copied()
            } else {
                break;
            };

            let was_removed = self
                .node_state
                .remove_if(&context_id, |(is_used, children_count)| {
                    !*is_used && *children_count == 0
                })
                .is_some();

            if was_removed {
                self.nodes.remove(&context_id);

                if let Some(p_id) = parent_id {
                    self.node_state
                        .update(&p_id, |_, (_, children)| *children -= 1);
                    context_id = p_id;
                    continue;
                }
            }
            break;
        }
    }
}
