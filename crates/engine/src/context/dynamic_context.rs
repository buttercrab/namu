use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use namu_core::{Value, ValueId};
use scc::ebr::Guard;
use scc::{HashIndex, HashMap as SccHashMap};

use crate::context::ContextManager;
#[derive(Clone, Debug)]
struct ContextTreeNode {
    ancestors: Vec<usize>,
    depth: usize,
    order: usize,
    val_id: ValueId,
    value: Value,
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

    fn new_node(&self, parent: Option<usize>, val_id: ValueId, value: Value) -> usize {
        let id = self.id_counter.fetch_add(1, Ordering::Relaxed);

        // Increment the parent's `children_count` *if* the parent is still alive.
        // It is possible that the parent was concurrently cleaned-up by another
        // thread between the time it was used and this call. In that scenario
        // simply skip the update – the parent node has already been reclaimed
        // and thus will not observe new children.
        if let Some(parent_id) = parent {
            // We intentionally ignore the return value here because the parent
            // entry may have been removed by a concurrent `remove_context`
            // sweep. Failing to find the parent is not fatal for correctness –
            // the only consequence is that the eventual clean-up of the parent
            // will not wait for this (already reclaimed) child.
            let _ = self.node_state.update(&parent_id, |_, (_, children)| {
                *children += 1;
            });
        }

        // Use the monotonically increasing `id` as the ordering key among
        // siblings. This is simpler than maintaining a separate per-parent
        // counter and is immune to concurrent removals.
        let order = id;

        let (depth, ancestors, segment_tree) = if let Some(parent_id) = parent {
            let guard = Guard::new();
            let mut ancestors = vec![parent_id];
            let mut i = 0;

            while let Some(p_ancestor) = self
                .nodes
                .peek(ancestors.last().unwrap(), &guard)
                .unwrap()
                .ancestors
                .get(i)
            {
                ancestors.push(*p_ancestor);
                i += 1;
            }

            let parent_node = self.nodes.peek(&parent_id, &guard).unwrap();
            let depth = parent_node.depth + 1;
            let segment_tree = parent_node.segment_tree.make(val_id, depth);

            (depth, ancestors, segment_tree)
        } else {
            (0, vec![], SegmentTree::new())
        };

        let node = ContextTreeNode {
            ancestors,
            depth,
            order,
            val_id,
            value,
            segment_tree,
        };
        // SAFETY: the `id` is unique across the lifetime of the process
        // because it comes from a single atomic counter.
        self.nodes.insert(id, node).unwrap();
        self.node_state.insert(id, (true, 0)).unwrap();
        id
    }

    fn ascend_to_depth<'a>(
        &'a self,
        mut context_id: usize,
        mut node: &'a ContextTreeNode,
        depth: usize,
        guard: &'a Guard,
    ) -> (usize, &'a ContextTreeNode) {
        let mut diff = node.depth - depth;
        let mut i = 1;
        let mut j = 0;

        while diff > 0 {
            if diff & i > 0 {
                context_id = node.ancestors[j];
                node = self.nodes.peek(&context_id, &guard).unwrap();
                diff ^= i;
            }
            i <<= 1;
            j += 1;
        }

        (context_id, node)
    }
}

impl ContextManager for DynamicContextManager {
    type ContextId = usize;

    fn create_context(&self) -> Self::ContextId {
        let id = self.new_node(None, 0, namu_core::Value::new(()));
        id
    }

    fn compare_context(&self, a: Self::ContextId, b: Self::ContextId) -> std::cmp::Ordering {
        if a == b {
            return std::cmp::Ordering::Equal;
        }

        let guard = Guard::new();

        let original_a = self.nodes.peek(&a, &guard).unwrap();
        let original_b = self.nodes.peek(&b, &guard).unwrap();
        let mut node_a = original_a;
        let mut node_b = original_b;
        let mut current_a = a;
        let mut current_b = b;

        // Equalize depths using binary lifting
        if node_a.depth > node_b.depth {
            (current_a, node_a) = self.ascend_to_depth(current_a, node_a, node_b.depth, &guard)
        } else if node_b.depth > node_a.depth {
            (current_b, node_b) = self.ascend_to_depth(current_b, node_b, node_a.depth, &guard)
        }

        if current_a == current_b {
            // One is an ancestor of the other. The original `a` or `b` with greater depth is
            // "larger".
            return original_a.depth.cmp(&original_b.depth);
        }

        // Move up together to find children of LCA
        for i in (0..node_a.ancestors.len()).rev() {
            let ancestor_a = node_a.ancestors.get(i);
            let ancestor_b = node_b.ancestors.get(i);
            if ancestor_a != ancestor_b {
                current_a = *ancestor_a.unwrap();
                current_b = *ancestor_b.unwrap();
                node_a = self.nodes.peek(&current_a, &guard).unwrap();
                node_b = self.nodes.peek(&current_b, &guard).unwrap();
            }
        }

        // Now current_a and current_b are the children of the LCA
        node_a.order.cmp(&node_b.order)
    }

    fn add_value(
        &self,
        context_id: Self::ContextId,
        val_id: ValueId,
        value: Value,
    ) -> Self::ContextId {
        self.new_node(Some(context_id), val_id, value)
    }

    fn get_value(&self, context_id: Self::ContextId, val_id: ValueId) -> Value {
        let guard = Guard::new();
        let node = self.nodes.peek(&context_id, &guard).unwrap();
        let depth = node.segment_tree.get(val_id);
        let (_, node) = self.ascend_to_depth(context_id, node, depth, &guard);
        return node.value.clone();
    }

    fn get_values(&self, context_id: Self::ContextId, val_ids: &[ValueId]) -> Vec<Value> {
        if val_ids.is_empty() {
            return vec![];
        }

        let guard = Guard::new();
        let start_node = self.nodes.peek(&context_id, &guard).unwrap();
        let mut found_vals = HashMap::with_capacity(val_ids.len());

        // Step 1 & 2: Get target depths and group by depth.
        let mut depths_to_visit: HashMap<usize, Vec<ValueId>> = HashMap::new();
        for &val_id in val_ids {
            if found_vals.contains_key(&val_id) {
                // Handle duplicates in input
                continue;
            }
            let depth = start_node.segment_tree.get(val_id);
            depths_to_visit.entry(depth).or_default().push(val_id);
        }

        // Step 3: Sort unique depths in descending order.
        let mut sorted_depths: Vec<_> = depths_to_visit.keys().copied().collect();
        sorted_depths.sort_unstable_by(|a, b| b.cmp(a)); // Sort descending

        // Step 4: Single upward traversal.
        let mut current_id = context_id;
        let mut current_node = start_node;

        for depth in sorted_depths {
            // Jump from the current position up to the target depth.
            let (next_id, next_node) =
                self.ascend_to_depth(current_id, current_node, depth, &guard);
            current_id = next_id;
            current_node = next_node;

            // Now we are at the correct ancestor node. Collect the valiable(s) defined here.
            if let Some(vals_at_this_depth) = depths_to_visit.get(&depth) {
                // The current node's val_id *should* be one of the ones we're looking for.
                if vals_at_this_depth.contains(&current_node.val_id) {
                    found_vals.insert(current_node.val_id, &current_node.value);
                }
            }
        }

        // Step 5: Format output to match original order.
        val_ids
            .iter()
            .map(|val_id| (*found_vals.get(val_id).unwrap()).clone())
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

        let guard = Guard::new();

        while let Some(parent_id) = self
            .nodes
            .peek(&context_id, &guard)
            .map(|node| node.ancestors.first().copied())
        {
            let was_removed = self
                .node_state
                .remove_if(&context_id, |(is_used, children_count)| {
                    !*is_used && *children_count == 0
                })
                .is_some();

            if was_removed {
                self.nodes.remove(&context_id);

                if let Some(p_id) = parent_id {
                    self.node_state.update(&p_id, |_, (_, children)| {
                        if *children > 0 {
                            *children -= 1;
                        }
                    });
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

        // After ensuring the range is sufficient, perform the update using an
        // iterative path-copy algorithm (non-recursive).

        // Each entry in the path keeps the range of the node we descended from,
        // a reference to the original node (if any), and whether the descent
        // went to the left child.
        let mut path: Vec<(usize, usize, Option<Arc<SegmentTreeNode>>, bool)> = Vec::new();
        let mut cur_begin = next_range_begin;
        let mut cur_end = next_range_end;
        let mut current = next_root.clone();

        // Descend to the target leaf, collecting the path along the way.
        while cur_begin != cur_end {
            let mid = cur_begin + (cur_end - cur_begin) / 2;
            if index <= mid {
                path.push((cur_begin, cur_end, current.clone(), true));
                current = current.as_ref().and_then(|n| n.left.clone());
                cur_end = mid;
            } else {
                path.push((cur_begin, cur_end, current.clone(), false));
                current = current.as_ref().and_then(|n| n.right.clone());
                cur_begin = mid + 1;
            }
        }

        // Create or override the leaf node with the provided value.
        let mut new_child = Arc::new(SegmentTreeNode {
            left: None,
            right: None,
            range_begin: cur_begin,
            range_end: cur_end,
            value: Some(value),
        });

        // Re-build the path back up, creating new internal nodes that reference
        // either the updated child or the reused sibling nodes.
        for (p_begin, p_end, orig, is_left) in path.into_iter().rev() {
            let (left_child, right_child) = if is_left {
                (
                    Some(new_child.clone()),
                    orig.as_ref().and_then(|n| n.right.clone()),
                )
            } else {
                (
                    orig.as_ref().and_then(|n| n.left.clone()),
                    Some(new_child.clone()),
                )
            };

            new_child = Arc::new(SegmentTreeNode {
                left: left_child,
                right: right_child,
                range_begin: p_begin,
                range_end: p_end,
                value: None,
            });
        }

        SegmentTree {
            root: Some(new_child),
            range_begin: next_range_begin,
            range_end: next_range_end,
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_tree_basic() {
        let tree = SegmentTree::new();
        let tree = tree.make(0, 1);
        assert_eq!(tree.get(0), 1);
        // Unset index should return 0
        assert_eq!(tree.get(1), 0);

        // Update another index, causing range expansion
        let tree = tree.make(5, 7);
        assert_eq!(tree.get(5), 7);
        // Existing value should persist due to persistence
        assert_eq!(tree.get(0), 1);

        // Update existing index, overriding value
        let tree2 = tree.make(0, 3);
        assert_eq!(tree2.get(0), 3);
        // Original tree remains unchanged
        assert_eq!(tree.get(0), 1);
    }

    #[test]
    fn dynamic_context_manager_vals() {
        let ctx_mgr = DynamicContextManager::new();
        let root = ctx_mgr.create_context();

        // Add val 1 to root context
        let ctx1 = ctx_mgr.add_value(root, 1, namu_core::Value::new(10usize));
        let val = *ctx_mgr.get_value(ctx1, 1).downcast_ref::<usize>().unwrap();
        assert_eq!(val, 10);

        // Add another val 2 in new context
        let ctx2 = ctx_mgr.add_value(ctx1, 2, namu_core::Value::new(20usize));
        let vals = ctx_mgr.get_values(ctx2, &[1, 2]);
        let v1 = *vals[0].downcast_ref::<usize>().unwrap();
        let v2 = *vals[1].downcast_ref::<usize>().unwrap();
        assert_eq!((v1, v2), (10, 20));

        // Compare contexts ordering (root < ctx1 < ctx2)
        assert_eq!(
            ctx_mgr.compare_context(root, ctx1),
            std::cmp::Ordering::Less
        );
        assert_eq!(
            ctx_mgr.compare_context(ctx1, ctx2),
            std::cmp::Ordering::Less
        );
    }
}
