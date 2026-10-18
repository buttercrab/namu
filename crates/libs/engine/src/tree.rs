//! The value tree.
//!
//! Every step that produces values creates a node holding them. Nodes are immutable, parent
//! included, so any worker may cache any node, and branches share everything written before they
//! split. A new node attaches under the deepest node of its context's chain that still holds a
//! live value (design §6.2), so nodes holding only dead values fall out of every chain.
//!
//! Each worker keeps the nodes it holds for a run in [`Nodes`], and sweeps away the ones nothing
//! here can reach any more: everything is reached from pinned contexts (branches being driven,
//! scopes waiting to continue) through parent links. A swept node that is asked for again is read
//! from the log, which holds every node a live branch can reach (design §7.2).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use namu_core::ValueId;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ids::NodeId;

#[derive(Debug, Serialize, Deserialize)]
pub struct Node {
    pub parent: Option<NodeId>,
    pub values: Vec<(ValueId, Value)>,
}

impl Node {
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

/// Nodes are never swept while the map is smaller than this.
const SWEEP_FLOOR: usize = 1024;

/// The nodes of one run held by one worker: those it created and those it fetched.
#[derive(Debug, Default)]
pub struct Nodes {
    inner: Mutex<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    /// Each node with the generation it arrived in.
    nodes: HashMap<NodeId, (Arc<Node>, u64)>,
    /// Pinned contexts, and how many pins each has.
    pins: HashMap<NodeId, usize>,
    /// Bumped by every sweep. A sweep keeps the nodes of the current generation, so a node just
    /// created is safe until whoever created it has pinned it or a child of it.
    generation: u64,
    /// Node count after the last sweep; the next one runs when it has doubled.
    after_sweep: usize,
    /// The most nodes held at once.
    peak: usize,
}

impl Nodes {
    pub fn get(&self, id: NodeId) -> Option<Arc<Node>> {
        let inner = self.inner.lock().unwrap();
        inner.nodes.get(&id).map(|(node, _)| node.clone())
    }

    pub fn insert(&self, id: NodeId, node: Arc<Node>) {
        let mut inner = self.inner.lock().unwrap();
        let generation = inner.generation;
        inner.nodes.insert(id, (node, generation));
        inner.peak = inner.peak.max(inner.nodes.len());
        if inner.nodes.len() >= SWEEP_FLOOR.max(2 * inner.after_sweep) {
            inner.sweep();
        }
    }

    /// Keeps `id`, and everything it reaches, until the pin is dropped.
    pub fn pin(self: &Arc<Self>, id: NodeId) -> Pin {
        *self.inner.lock().unwrap().pins.entry(id).or_default() += 1;
        Pin {
            nodes: self.clone(),
            id,
        }
    }

    fn unpin(&self, id: NodeId) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(count) = inner.pins.get_mut(&id) {
            *count -= 1;
            if *count == 0 {
                inner.pins.remove(&id);
            }
        }
    }

    /// The most nodes held at once so far.
    pub fn peak(&self) -> usize {
        self.inner.lock().unwrap().peak
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Drops every node nothing pinned reaches, except those of the current generation.
    pub fn sweep(&self) {
        self.inner.lock().unwrap().sweep();
    }
}

impl Inner {
    fn sweep(&mut self) {
        let mut reached: HashSet<NodeId> = HashSet::new();
        for &pinned in self.pins.keys() {
            let mut cursor = Some(pinned);
            while let Some(id) = cursor {
                if !reached.insert(id) {
                    break;
                }
                cursor = self.nodes.get(&id).and_then(|(node, _)| node.parent);
            }
        }
        let current = self.generation;
        self.nodes
            .retain(|id, (_, generation)| *generation == current || reached.contains(id));
        self.generation += 1;
        self.after_sweep = self.nodes.len();
    }
}

/// A pinned context: it and its ancestors stay held while the pin lives.
#[derive(Debug)]
pub struct Pin {
    nodes: Arc<Nodes>,
    id: NodeId,
}

impl Pin {
    pub fn id(&self) -> NodeId {
        self.id
    }

    /// Moves the pin to `id`.
    pub fn set(&mut self, id: NodeId) {
        if id != self.id {
            let next = self.nodes.pin(id);
            drop(std::mem::replace(self, next));
        }
    }
}

impl Drop for Pin {
    fn drop(&mut self) {
        self.nodes.unpin(self.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(parent: Option<NodeId>) -> Arc<Node> {
        Arc::new(Node {
            parent,
            values: Vec::new(),
        })
    }

    #[test]
    fn a_sweep_keeps_what_pins_reach_and_what_is_new() {
        let nodes = Arc::new(Nodes::default());
        nodes.insert(1, node(None));
        nodes.insert(2, node(Some(1)));
        nodes.insert(3, node(Some(1)));
        let pin = nodes.pin(2);
        // Everything is of the current generation: nothing goes yet.
        nodes.sweep();
        assert_eq!(nodes.len(), 3);
        // Now 3 is old and unreachable; 1 and 2 are pinned through 2.
        nodes.sweep();
        assert!(nodes.get(3).is_none());
        assert!(nodes.get(1).is_some() && nodes.get(2).is_some());
        drop(pin);
        nodes.sweep();
        assert!(nodes.is_empty());
    }
}
