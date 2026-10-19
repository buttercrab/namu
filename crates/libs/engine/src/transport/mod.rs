//! How messages reach other workers.

mod local;
mod tcp;

pub use local::LocalTransport;
pub use tcp::TcpTransport;

use crate::ids::WorkerId;
use crate::message::Message;

/// Delivers messages to workers: exactly once and in order per pair of workers, for as long as
/// both live.
pub trait Transport: Send + Sync + 'static {
    fn send(&self, to: WorkerId, message: Message);
}
