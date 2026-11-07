//! How messages reach other workers.

#[cfg(any(test, feature = "certs"))]
pub mod certs;
mod local;
mod tcp;
mod tls;

pub use local::LocalTransport;
pub use tcp::TcpTransport;
pub use tls::{MESH_NAME, MeshTls};

use crate::ids::WorkerId;
use crate::message::Message;

/// Delivers messages to workers: exactly once and in order per pair of workers, for as long as
/// both live.
pub trait Transport: Send + Sync + 'static {
    fn send(&self, to: WorkerId, message: Message);
}
