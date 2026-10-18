//! How messages reach other workers.

mod local;
mod tcp;

pub use local::LocalTransport;
pub use tcp::TcpTransport;

use crate::ids::WorkerId;
use crate::message::Message;

/// Delivers messages to workers, in order per pair of workers.
pub trait Transport: Send + Sync + 'static {
    fn send(&self, to: WorkerId, message: Message);
}
