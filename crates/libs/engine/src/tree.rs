//! The value tree.
//!
//! Every step that produces values creates a node holding them. Nodes are immutable, so any
//! worker may cache any node, and branches share everything written before they split. A new node
//! attaches under the deepest node of its context's chain that still holds a live value (design
//! §6.2), so nodes holding only dead values fall out of every chain.
//!
//! A node owns its parent: a context keeps its whole chain alive, and a node nothing reaches is
//! freed at once. A lookup takes O(log depth): each node knows the depth of the node defining each
//! value it sees (a persistent segment tree, shared with its parent's) and its ancestors 2^i up
//! (binary lifting), so it jumps straight to the node holding the value (design §6.1). A node that
//! came from another worker knows only its parent's id until the parent is held here too; from then
//! on it owns it like any other.
//!
//! Each worker indexes the nodes it holds for a run in [`Nodes`], without owning them, to serve
//! them to other workers and to share one copy of each. A node asked for after it was freed is
//! read from the log, which holds every node a live branch can reach (design §7.2).
//!
//! A node crosses a process boundary as MessagePack, encoded the first time it does and kept, so
//! a node shipped to several workers and logged is encoded once. A node that arrived encoded
//! keeps the bytes it came as.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, Weak};

use bytes::Bytes;
use namu_core::ValueId;
use namu_core::value::Value;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::ids::NodeId;
use crate::persistent::PersistentMap;

pub struct Node {
    id: NodeId,
    parent: Option<Parent>,
    values: Vec<(ValueId, Value)>,
    /// Set once the whole chain is held here: at creation for a node made here, when its chain
    /// arrives for one that came from elsewhere.
    lineage: OnceLock<Lineage>,
    /// The node as MessagePack, once it has been encoded or arrived encoded.
    encoded: OnceLock<Bytes>,
}

/// Where a node sits in its chain.
struct Lineage {
    depth: usize,
    /// The ancestors 2^i up.
    jumps: Vec<Arc<Node>>,
    /// For each value this node sees, the depth of the node that defines it.
    defs: PersistentMap<usize>,
}

impl Lineage {
    fn root(values: &[(ValueId, Value)]) -> Self {
        Self {
            depth: 0,
            jumps: Vec::new(),
            defs: Self::define(PersistentMap::default(), values, 0),
        }
    }

    /// The lineage of a node holding `values` under `parent`, whose lineage is `above`.
    fn under(parent: &Arc<Node>, above: &Lineage, values: &[(ValueId, Value)]) -> Self {
        let mut jumps = vec![parent.clone()];
        while let Some(next) = jumps[jumps.len() - 1]
            .lineage
            .get()
            .and_then(|lineage| lineage.jumps.get(jumps.len() - 1))
        {
            jumps.push(next.clone());
        }
        let depth = above.depth + 1;
        Self {
            depth,
            jumps,
            defs: Self::define(above.defs.clone(), values, depth),
        }
    }

    fn define(
        defs: PersistentMap<usize>,
        values: &[(ValueId, Value)],
        depth: usize,
    ) -> PersistentMap<usize> {
        values
            .iter()
            .fold(defs, |defs, (id, _)| defs.insert(*id, depth))
    }
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
            lineage: OnceLock::from(Lineage::root(&[])),
            encoded: OnceLock::new(),
        }
    }

    /// A node holding `values` under `parent`.
    pub fn new(id: NodeId, parent: Arc<Node>, values: Vec<(ValueId, Value)>) -> Self {
        let lineage = OnceLock::new();
        if let Some(above) = parent.lineage() {
            let _ = lineage.set(Lineage::under(&parent, above, &values));
        }
        Self {
            id,
            parent: Some(Parent {
                id: parent.id,
                node: OnceLock::from(parent),
            }),
            values,
            lineage,
            encoded: OnceLock::new(),
        }
    }

    /// The node's lineage, built (with any ancestors' missing ones) if its chain is held here.
    fn lineage(&self) -> Option<&Lineage> {
        if let Some(lineage) = self.lineage.get() {
            return Some(lineage);
        }
        if self.parent.is_none() {
            return Some(self.lineage.get_or_init(|| Lineage::root(&self.values)));
        }
        // The ancestors without a lineage, nearest first, up to one that has one.
        let mut pending = Vec::new();
        let mut cursor = self.parent()?;
        while cursor.lineage.get().is_none() {
            if cursor.parent.is_none() {
                cursor.lineage.get_or_init(|| Lineage::root(&cursor.values));
                break;
            }
            pending.push(cursor);
            cursor = cursor.parent()?;
        }
        for node in pending.into_iter().rev() {
            node.lineage_under_parent();
        }
        self.lineage_under_parent();
        self.lineage.get()
    }

    /// Builds the lineage from the parent's, which is built.
    fn lineage_under_parent(&self) {
        if let Some(parent) = self.parent()
            && let Some(above) = parent.lineage.get()
        {
            self.lineage
                .get_or_init(|| Lineage::under(parent, above, &self.values));
        }
    }

    /// `value` as seen from this node, found in O(log depth): `None` if this node's chain is not
    /// all held here yet, `Some(None)` if the value is not visible from it.
    pub fn find(&self, value: ValueId) -> Option<Option<&Value>> {
        let lineage = self.lineage()?;
        let Some(depth) = lineage.defs.get(value) else {
            return Some(None);
        };
        let (mut node, mut up, mut bit) = (self, lineage.depth - depth, 0);
        while up > 0 {
            if up & 1 == 1 {
                node = &node.lineage.get().expect("a chain with a lineage").jumps[bit];
            }
            up >>= 1;
            bit += 1;
        }
        Some(node.get(value))
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
    /// no chain is too long to drop. A node's jumps go first: they only point further up the
    /// chain its parent already holds.
    fn drop(&mut self) {
        self.lineage.take();
        let mut next = self
            .parent
            .take()
            .and_then(|parent| parent.node.into_inner());
        while let Some(node) = next {
            next = Arc::into_inner(node)
                .and_then(|mut node| {
                    node.lineage.take();
                    node.parent.take()
                })
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

impl Node {
    /// The node as MessagePack, encoded on first use.
    fn encoded(&self) -> Result<&Bytes, rmp_serde::encode::Error> {
        if let Some(encoded) = self.encoded.get() {
            return Ok(encoded);
        }
        let wire = WireRef {
            id: self.id,
            parent: self.parent_id(),
            values: &self.values,
        };
        let encoded = rmp_serde::to_vec_named(&wire)?;
        Ok(self.encoded.get_or_init(|| encoded.into()))
    }
}

impl Serialize for Node {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let encoded = self.encoded().map_err(serde::ser::Error::custom)?;
        serializer.serialize_bytes(encoded)
    }
}

impl<'de> Deserialize<'de> for Node {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let encoded = serde_bytes::ByteBuf::deserialize(deserializer)?.into_vec();
        let wire: Wire = rmp_serde::from_slice(&encoded).map_err(serde::de::Error::custom)?;
        Ok(Self {
            id: wire.id,
            parent: wire.parent.map(|id| Parent {
                id,
                node: OnceLock::new(),
            }),
            values: wire.values,
            lineage: OnceLock::new(),
            encoded: OnceLock::from(Bytes::from(encoded)),
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
    fn a_node_is_encoded_once_however_often_it_is_sent() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        static ENCODED: AtomicUsize = AtomicUsize::new(0);
        #[derive(Clone, Deserialize)]
        struct Counted(i32);
        impl Serialize for Counted {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                ENCODED.fetch_add(1, Ordering::Relaxed);
                self.0.serialize(serializer)
            }
        }
        impl namu_core::value::Data for Counted {}

        let root = Arc::new(Node::root(1));
        let node = Node::new(2, root, vec![(0, Value::new(Counted(7)))]);
        let first = rmp_serde::to_vec(&node).unwrap();
        let second = rmp_serde::to_vec(&node).unwrap();
        assert_eq!(first, second);
        // A node that arrived goes on as the bytes it came as.
        let arrived: Node = rmp_serde::from_slice(&first).unwrap();
        assert_eq!(rmp_serde::to_vec(&arrived).unwrap(), first);
        assert_eq!(ENCODED.load(Ordering::Relaxed), 1);
        assert_eq!(arrived.get(0).unwrap().to::<Counted>().unwrap().0, 7);
    }

    #[test]
    fn a_long_chain_drops_without_recursion() {
        let mut chain = Arc::new(Node::root(0));
        for id in 1..300_000 {
            chain = Arc::new(Node::new(id, chain, Vec::new()));
        }
        drop(chain);
    }

    /// Walks the chain one parent at a time: what `find` must agree with.
    fn walk(node: &Node, value: ValueId) -> Option<i64> {
        let mut cursor = Some(node);
        while let Some(node) = cursor {
            if let Some(found) = node.get(value) {
                return Some(found.to().unwrap());
            }
            cursor = node.parent().map(Arc::as_ref);
        }
        None
    }

    #[test]
    fn lookups_jump_to_the_defining_node() {
        // A chain of 2,000 nodes, each defining a few of 50 values, so most values are
        // redefined (shadowed) many times.
        let mut chain = Arc::new(Node::root(0));
        let mut seed = 1u64;
        for id in 1..2000 {
            let values = (0..3)
                .map(|_| {
                    seed = seed
                        .wrapping_mul(6364136223846793005)
                        .wrapping_add(1442695040888963407);
                    ((seed >> 33) as usize % 50, Value::new(id as i64))
                })
                .collect();
            chain = Arc::new(Node::new(id, chain, values));
        }
        let mut node: &Node = &chain;
        for _ in 0..300 {
            for value in 0..55 {
                let found = node.find(value).expect("a chain made here has a lineage");
                assert_eq!(found.map(|v| v.to::<i64>().unwrap()), walk(node, value));
            }
            node = node.parent().unwrap();
        }
    }

    #[test]
    fn a_chain_that_arrived_finds_once_it_is_held() {
        let root = Arc::new(Node::root(1));
        let a = Arc::new(Node::new(2, root.clone(), vec![(7, Value::new(1i64))]));
        let b = Node::new(3, a.clone(), vec![(8, Value::new(2i64))]);
        let wire = |node: &Node| -> Node {
            rmp_serde::from_slice(&rmp_serde::to_vec(node).unwrap()).unwrap()
        };
        let (a, b) = (Arc::new(wire(&a)), wire(&b));
        assert!(b.find(7).is_none(), "the chain is not held yet");
        b.link(a.clone());
        assert!(
            b.find(7).is_none(),
            "the chain is still not held to its root"
        );
        a.link(root);
        assert_eq!(b.find(7).unwrap().unwrap().to::<i64>().unwrap(), 1);
        assert_eq!(b.find(8).unwrap().unwrap().to::<i64>().unwrap(), 2);
        assert!(b.find(9).unwrap().is_none());
    }
}
