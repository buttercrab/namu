//! The value tree.
//!
//! Every step that produces values creates a node holding them, under the context it ran in.
//! Nodes are immutable, parent included, so any worker may cache any node forever, and
//! branches share everything written before they split.

use std::collections::HashMap;
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
}

/// The nodes of one run held by one worker: those it created and those it fetched.
#[derive(Debug, Default)]
pub struct Nodes {
    nodes: Mutex<HashMap<NodeId, Arc<Node>>>,
}

impl Nodes {
    pub fn get(&self, id: NodeId) -> Option<Arc<Node>> {
        self.nodes.lock().unwrap().get(&id).cloned()
    }

    pub fn insert(&self, id: NodeId, node: Arc<Node>) {
        self.nodes.lock().unwrap().insert(id, node);
    }
}
