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

/// A branch's logical identity: a hash of its path through the run's fan-outs, from the root
/// branch (lineage 0). Unlike its [`BranchId`](crate::message::BranchId), which is minted, it is
/// the same each time the engine runs the same part of the run again, wherever that is.
pub type Lineage = u64;

/// The lineage of item `index` of what a branch with lineage `parent` fanned out (or streamed)
/// when its clock was at `clock`.
pub fn child_lineage(parent: Lineage, clock: u64, index: u64) -> Lineage {
    [clock, index]
        .into_iter()
        .fold(parent, |hash, word| mix(hash ^ mix(word)))
}

/// SplitMix64's finalizer: a fixed bijection that spreads every bit of `x` over the result.
fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

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
    fn a_lineage_depends_on_its_whole_path() {
        let a = child_lineage(0, 3, 7);
        assert_eq!(a, child_lineage(0, 3, 7));
        assert_ne!(a, child_lineage(0, 3, 8));
        assert_ne!(a, child_lineage(0, 4, 7));
        assert_ne!(a, child_lineage(1, 3, 7));
        // Swapping the clock and the index gives another path.
        assert_ne!(child_lineage(0, 1, 2), child_lineage(0, 2, 1));
        // Fixed across builds and machines: it names calls in the outside world.
        assert_eq!(child_lineage(0, 0, 0), 0xa7f7_2697_a273_1486);
    }

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
