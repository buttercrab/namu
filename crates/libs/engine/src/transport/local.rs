//! Workers in one process, connected by channels.

use std::collections::HashMap;
use std::sync::RwLock;

use tokio::sync::mpsc;

use super::Transport;
use crate::ids::WorkerId;
use crate::message::Message;

/// Messages are moved, never serialized.
#[derive(Debug, Default)]
pub struct LocalTransport {
    inboxes: RwLock<HashMap<WorkerId, mpsc::UnboundedSender<Message>>>,
}

impl LocalTransport {
    /// Adds a worker and returns its inbox.
    pub fn connect(&self, worker: WorkerId) -> mpsc::UnboundedReceiver<Message> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.inboxes.write().unwrap().insert(worker, tx);
        rx
    }
}

impl Transport for LocalTransport {
    fn send(&self, to: WorkerId, message: Message) {
        let inboxes = self.inboxes.read().unwrap();
        match inboxes.get(&to) {
            Some(inbox) if inbox.send(message).is_ok() => {}
            _ => tracing::error!(to, "message to an unknown or stopped worker dropped"),
        }
    }
}
