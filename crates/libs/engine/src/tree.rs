//! The value tree.
//!
//! Every step that produces values creates a node holding them. Nodes are immutable, so any
//! worker may cache any node, and branches share everything written before they split. A new node
//! attaches under the deepest node of its context's chain that still holds a live value (design
//! §6.2), so nodes holding only dead values fall out of every chain.
//!
//! A node owns its parent: a context keeps its whole chain alive, and a node nothing reaches is
//! freed at once. A node that came from another worker knows only its parent's id until the
//! parent is held here too; from then on it owns it like any other.
//!
//! Each worker indexes the nodes it holds for a run in [`Nodes`], without owning them, to serve
//! them to other workers and to share one copy of each. A node asked for after it was freed is
//! read from the log, which holds every node a live branch can reach (design §7.2).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, Weak};

use namu_core::ValueId;
use namu_core::value::Value;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::ids::NodeId;

pub struct Node {
    id: NodeId,
    parent: Option<Parent>,
    values: Vec<(ValueId, Value)>,
}

struct Parent {
    id: NodeId,
    /// Set once the parent is held; from then on this node keeps it alive.
    node: OnceLock<Arc<Node>>,
}

impl Node {
    pub fn root(id: NodeId) -> Self {
        Self {
            id,
            parent: None,
            values: Vec::new(),
        }
    }

    /// A node holding `values` under `parent`.
    pub fn new(id: NodeId, parent: Arc<Node>, values: Vec<(ValueId, Value)>) -> Self {
        Self {
            id,
            parent: Some(Parent {
                id: parent.id,
                node: OnceLock::from(parent),
            }),
            values,
        }
    }

    pub fn id(&self) -> NodeId {
        self.id
    }

    pub fn parent_id(&self) -> Option<NodeId> {
        self.parent.as_ref().map(|parent| parent.id)
    }

    /// The parent, if it is held here.
    pub fn parent(&self) -> Option<&Arc<Node>> {
        self.parent.as_ref()?.node.get()
    }

    /// Takes hold of the parent, which this node knew only by id.
    pub fn link(&self, parent: Arc<Node>) {
        let link = self.parent.as_ref().expect("linking a root");
        assert_eq!(link.id, parent.id, "linking node {} to a stranger", self.id);
        let _ = link.node.set(parent);
    }

    pub fn get(&self, id: ValueId) -> Option<&Value> {
        self.values
            .iter()
            .find_map(|(value_id, value)| (*value_id == id).then_some(value))
    }

    /// Whether the node holds one of `live` (sorted).
    pub fn holds_any(&self, live: &[ValueId]) -> bool {
        self.values
            .iter()
            .any(|(id, _)| live.binary_search(id).is_ok())
    }
}

impl Drop for Node {
    /// Frees the chain this node was the last owner of one node at a time, not recursively, so
    /// no chain is too long to drop.
    fn drop(&mut self) {
        let mut next = self
            .parent
            .take()
            .and_then(|parent| parent.node.into_inner());
        while let Some(node) = next {
            next = Arc::into_inner(node)
                .and_then(|mut node| node.parent.take())
                .and_then(|parent| parent.node.into_inner());
        }
    }
}

impl std::fmt::Debug for Node {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Node")
            .field("id", &self.id)
            .field("parent", &self.parent_id())
            .field("values", &self.values)
            .finish()
    }
}

/// A node as it crosses the network and sits in the log: its parent by id.
#[derive(Serialize)]
struct WireRef<'a> {
    id: NodeId,
    parent: Option<NodeId>,
    values: &'a [(ValueId, Value)],
}

#[derive(Deserialize)]
struct Wire {
    id: NodeId,
    parent: Option<NodeId>,
    values: Vec<(ValueId, Value)>,
}

impl Serialize for Node {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let wire = WireRef {
            id: self.id,
            parent: self.parent_id(),
            values: &self.values,
        };
        wire.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Node {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = Wire::deserialize(deserializer)?;
        Ok(Self {
            id: wire.id,
            parent: wire.parent.map(|id| Parent {
                id,
                node: OnceLock::new(),
            }),
            values: wire.values,
        })
    }
}

/// The index is never cleaned while it is smaller than this.
const CLEAN_FLOOR: usize = 64;

/// The nodes of one run that one worker holds, by id. The index does not keep them alive.
#[derive(Debug, Default)]
pub struct Nodes {
    inner: Mutex<Index>,
}

#[derive(Debug, Default)]
struct Index {
    nodes: HashMap<NodeId, Weak<Node>>,
    /// Entries left after the last cleaning; the next one runs when that has doubled, so
    /// cleaning costs O(1) per entry.
    after_clean: usize,
    /// The most entries at once.
    peak: usize,
}

impl Nodes {
    pub fn get(&self, id: NodeId) -> Option<Arc<Node>> {
        self.inner.lock().unwrap().nodes.get(&id)?.upgrade()
    }

    /// Indexes `node`, and returns the copy to use: the one already held, if there is one.
    pub fn hold(&self, node: Arc<Node>) -> Arc<Node> {
        let mut index = self.inner.lock().unwrap();
        if let Some(held) = index.nodes.get(&node.id).and_then(Weak::upgrade) {
            return held;
        }
        index.nodes.insert(node.id, Arc::downgrade(&node));
        index.peak = index.peak.max(index.nodes.len());
        if index.nodes.len() >= CLEAN_FLOOR.max(2 * index.after_clean) {
            index.nodes.retain(|_, node| node.strong_count() > 0);
            index.after_clean = index.nodes.len();
        }
        node
    }

    /// Indexes nodes that arrived together, and links each to its parent if that is held.
    pub fn receive(&self, nodes: impl IntoIterator<Item = Arc<Node>>) -> Vec<Arc<Node>> {
        let nodes: Vec<_> = nodes.into_iter().map(|node| self.hold(node)).collect();
        for node in &nodes {
            if node.parent().is_none()
                && let Some(parent) = node.parent_id().and_then(|id| self.get(id))
            {
                node.link(parent);
            }
        }
        nodes
    }

    /// The most index entries at once: a bound on the nodes held at once.
    pub fn peak(&self) -> usize {
        self.inner.lock().unwrap().peak
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_context_keeps_its_chain_and_nothing_else() {
        let nodes = Nodes::default();
        let root = nodes.hold(Arc::new(Node::root(1)));
        let kept = nodes.hold(Arc::new(Node::new(2, root.clone(), Vec::new())));
        let dropped = nodes.hold(Arc::new(Node::new(3, root, Vec::new())));
        drop(dropped);
        assert!(nodes.get(3).is_none());
        assert!(nodes.get(1).is_some() && nodes.get(2).is_some());
        drop(kept);
        assert!(nodes.get(1).is_none());
    }

    #[test]
    fn nodes_that_arrive_link_to_their_parents() {
        let nodes = Nodes::default();
        let root = nodes.hold(Arc::new(Node::root(1)));
        let wire = rmp_serde::to_vec_named(&Node::new(2, root.clone(), Vec::new())).unwrap();
        let child: Node = rmp_serde::from_slice(&wire).unwrap();
        assert!(child.parent().is_none());
        let child = nodes.receive([Arc::new(child)]).remove(0);
        assert!(Arc::ptr_eq(child.parent().unwrap(), &root));
    }

    #[test]
    fn a_long_chain_drops_without_recursion() {
        let mut chain = Arc::new(Node::root(0));
        for id in 1..1_000_000 {
            chain = Arc::new(Node::new(id, chain, Vec::new()));
        }
        drop(chain);
    }
}
