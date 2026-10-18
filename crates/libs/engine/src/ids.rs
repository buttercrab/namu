//! Identifiers minted by workers without coordination.

use std::sync::atomic::{AtomicU64, Ordering};

/// A worker in a cluster. 0 is reserved for the control plane.
pub type WorkerId = u64;

/// A value-tree node. Its high bits hold the worker that created it, which holds the node and
/// serves it to others, so no directory is needed to find it.
pub type NodeId = u64;

/// A fan-out scope, minted like a node id; the minting worker owns the scope.
pub type ScopeId = u64;

pub type RunId = uuid::Uuid;

const COUNTER_BITS: u32 = 40;

/// The worker that minted `id`.
pub fn owner(id: u64) -> WorkerId {
    id >> COUNTER_BITS
}

/// Mints node and scope ids for one worker.
#[derive(Debug)]
pub struct Minter {
    worker: WorkerId,
    counter: AtomicU64,
}

impl Minter {
    pub fn new(worker: WorkerId) -> Self {
        Self {
            worker,
            counter: AtomicU64::new(0),
        }
    }

    /// The next id this minter will give out; every id given out later is at least this.
    pub fn peek(&self) -> u64 {
        (self.worker << COUNTER_BITS) | self.counter.load(Ordering::Relaxed)
    }

    pub fn next(&self) -> u64 {
        let counter = self.counter.fetch_add(1, Ordering::Relaxed);
        assert!(
            counter < 1 << COUNTER_BITS,
            "worker {} ran out of ids",
            self.worker
        );
        (self.worker << COUNTER_BITS) | counter
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_name_their_owner() {
        let a = Minter::new(1);
        let b = Minter::new(2);
        let (a1, a2, b1) = (a.next(), a.next(), b.next());
        assert_ne!(a1, a2);
        assert_ne!(a1, b1);
        assert_eq!(owner(a2), 1);
        assert_eq!(owner(b1), 2);
    }
}
