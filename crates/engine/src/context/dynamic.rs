use std::{
    any::Any,
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use common::VarId;
use scc::{HashIndex, HashMap as SccHashMap, hash_index::OccupiedEntry};

use crate::context::ContextManager;

#[derive(Clone, Debug)]
struct ContextTreeNode {
    ancestors: Vec<usize>,
    depth: usize,
    order: usize,
    var_id: VarId,
    value: Arc<dyn Any + Send + Sync>,
    segment_tree: SegmentTree,
}

pub struct DynamicContextManager {
    nodes: HashIndex<usize, ContextTreeNode>,
    node_state: SccHashMap<usize, (bool, usize)>, // (is_used, children_count)
    id_counter: AtomicUsize,
}

impl DynamicContextManager {
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

        let (depth, ancestors, segment_tree) = if let Some(parent_id) = parent {
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

            let depth = parent_node.depth + 1;
            let segment_tree = parent_node.segment_tree.make(var_id, depth);

            (depth, ancestors, segment_tree)
        } else {
            (0, vec![], SegmentTree::new())
        };

        let node = ContextTreeNode {
            ancestors,
            depth,
            order,
            var_id,
            value,
            segment_tree,
        };
        let id = self.id_counter.fetch_add(1, Ordering::Relaxed);
        self.nodes.insert(id, node).unwrap();
        self.node_state.insert(id, (true, 0)).unwrap();
        id
    }

    fn go_up_to<'a>(
        &'a self,
        mut context_id: usize,
        mut node: OccupiedEntry<'a, usize, ContextTreeNode>,
        depth: usize,
    ) -> (usize, OccupiedEntry<'a, usize, ContextTreeNode>) {
        let mut diff = node.depth - depth;
        let mut i = 1;

        while diff > 0 {
            if diff & i > 0 {
                context_id = node.ancestors[i];
                node = self.nodes.get(&context_id).unwrap();
                diff ^= i;
            }
            i <<= 1;
        }

        (context_id, node)
    }
}

impl ContextManager for DynamicContextManager {
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
            (current_a, node_a) = self.go_up_to(current_a, node_a, node_b.depth)
        } else if node_b.depth > node_a.depth {
            (current_b, node_b) = self.go_up_to(current_b, node_b, node_a.depth)
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
    ) -> Arc<dyn Any + Send + Sync> {
        let node = self.nodes.get(&context_id).unwrap();
        let depth = node.segment_tree.get(var_id);
        let (_, node) = self.go_up_to(context_id, node, depth);
        return node.value.clone();
    }

    fn get_variables(
        &self,
        context_id: Self::ContextId,
        var_ids: &[VarId],
    ) -> Vec<Arc<dyn Any + Send + Sync>> {
        if var_ids.is_empty() {
            return vec![];
        }

        let start_node = self.nodes.get(&context_id).unwrap();
        let mut found_vars = HashMap::with_capacity(var_ids.len());

        // Step 1 & 2: Get target depths and group by depth.
        let mut depths_to_visit: HashMap<usize, Vec<VarId>> = HashMap::new();
        for &var_id in var_ids {
            if found_vars.contains_key(&var_id) {
                // Handle duplicates in input
                continue;
            }
            let depth = start_node.segment_tree.get(var_id);
            depths_to_visit.entry(depth).or_default().push(var_id);
        }

        // Step 3: Sort unique depths in descending order.
        let mut sorted_depths: Vec<_> = depths_to_visit.keys().copied().collect();
        sorted_depths.sort_unstable_by(|a, b| b.cmp(a)); // Sort descending

        // Step 4: Single upward traversal.
        let mut current_id = context_id;
        let mut current_node = start_node;

        for depth in sorted_depths {
            // Jump from the current position up to the target depth.
            let (next_id, next_node) = self.go_up_to(current_id, current_node, depth);
            current_id = next_id;
            current_node = next_node;

            // Now we are at the correct ancestor node. Collect the variable(s) defined here.
            if let Some(vars_at_this_depth) = depths_to_visit.get(&depth) {
                // The current node's var_id *should* be one of the ones we're looking for.
                if vars_at_this_depth.contains(&current_node.var_id) {
                    found_vars.insert(current_node.var_id, current_node.value.clone());
                }
            }
        }

        // Step 5: Format output to match original order.
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

// thread safe dynamic range persistent segment tree

#[derive(Debug, Clone)]
struct SegmentTreeNode {
    left: Option<Arc<SegmentTreeNode>>,
    right: Option<Arc<SegmentTreeNode>>,
    range_begin: usize,
    range_end: usize,
    value: Option<usize>, // Some if range_begin == range_end
}

#[derive(Debug, Clone)]
struct SegmentTree {
    root: Option<Arc<SegmentTreeNode>>,
    range_begin: usize,
    range_end: usize,
}

impl SegmentTree {
    fn new() -> SegmentTree {
        SegmentTree {
            root: None,
            range_begin: 0,
            range_end: 0,
        }
    }

    fn make(&self, index: usize, value: usize) -> SegmentTree {
        let mut next_root = self.root.clone();
        let mut next_range_end = self.range_end;
        let next_range_begin = self.range_begin;

        // Dynamically expand the tree's range until it can contain the index.
        while index > next_range_end {
            let old_root = next_root;
            let old_range_end = next_range_end;

            // Double the size of the range. e.g., [0, 7] -> [0, 15]
            // New size = (old_size) * 2
            // New end = begin + new_size - 1 = 0 + (old_end + 1) * 2 - 1 = 2 * old_end + 1
            next_range_end = old_range_end * 2 + 1;

            // Create a new root for the expanded range.
            // The entire old tree becomes the left child of the new root.
            let new_root_node = SegmentTreeNode {
                left: old_root,
                right: None, // The right half of the new range is empty initially.
                range_begin: next_range_begin,
                range_end: next_range_end,
                value: None,
            };
            next_root = Some(Arc::new(new_root_node));
        }

        // After ensuring the range is sufficient, perform the update recursively.
        let final_root =
            Self::make_recursive(&next_root, next_range_begin, next_range_end, index, value);

        SegmentTree {
            root: Some(final_root),
            range_begin: next_range_begin,
            range_end: next_range_end,
        }
    }

    fn make_recursive(
        node: &Option<Arc<SegmentTreeNode>>,
        range_begin: usize,
        range_end: usize,
        index: usize,
        value: usize,
    ) -> Arc<SegmentTreeNode> {
        // Base case: If we've reached the leaf's range, create the leaf node.
        if range_begin == range_end {
            return Arc::new(SegmentTreeNode {
                left: None,
                right: None,
                range_begin,
                range_end,
                value: Some(value),
            });
        }

        let mid = range_begin + (range_end - range_begin) / 2;

        // Recursively update the appropriate child and reuse the other one.
        let (new_left, new_right) = if index <= mid {
            // Recurse left, reusing the right child.
            (
                Some(Self::make_recursive(
                    &node.as_ref().and_then(|n| n.left.clone()),
                    range_begin,
                    mid,
                    index,
                    value,
                )),
                node.as_ref().and_then(|n| n.right.clone()),
            )
        } else {
            // Recurse right, reusing the left child.
            (
                node.as_ref().and_then(|n| n.left.clone()),
                Some(Self::make_recursive(
                    &node.as_ref().and_then(|n| n.right.clone()),
                    mid + 1,
                    range_end,
                    index,
                    value,
                )),
            )
        };

        // Create a new internal node pointing to the new/reused children.
        Arc::new(SegmentTreeNode {
            left: new_left,
            right: new_right,
            range_begin,
            range_end,
            value: None,
        })
    }

    fn get(&self, index: usize) -> usize {
        if index > self.range_end {
            return 0;
        }

        let mut current_node = self.root.as_ref();

        // Traverse the tree to find the leaf node for the given index.
        while let Some(node) = current_node {
            if node.range_begin == node.range_end {
                return node.value.unwrap_or(0);
            }

            let mid = node.range_begin + (node.range_end - node.range_begin) / 2;

            if index <= mid {
                current_node = node.left.as_ref();
            } else {
                current_node = node.right.as_ref();
            }
        }

        // If a node is not found along the path, the index was never set.
        0
    }
}
