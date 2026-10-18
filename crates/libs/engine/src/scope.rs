//! Fan-out scopes and how they end: weight throwing.
//!
//! A scope starts with [`FULL`] weight, split among its branches when they are created. Weight is
//! conserved: a branch passes its weight along with its work, and returns it to the scope's owner
//! when it ends, together with its collected item if it reached the collect. The scope is over
//! once the owner holds the full weight again. Creating branches never needs a message to the
//! owner, so no message ordering between workers can make a scope end early.

use std::sync::Arc;

use namu_core::OpId;
use serde_json::Value;
use tokio::sync::oneshot;

use crate::ids::{NodeId, ScopeId};
use crate::run::RunOutcome;

pub type Weight = u64;

/// The weight of a whole scope. Shares are integers, so accounting is exact; a scope can have up
/// to this many branches.
pub const FULL: Weight = 1 << 62;

/// Splits `weight` into `n` positive shares that sum to it exactly.
pub fn split(weight: Weight, n: usize) -> impl Iterator<Item = Weight> {
    let n = n as Weight;
    assert!(
        0 < n && n <= weight,
        "cannot split weight {weight} {n} ways"
    );
    let (share, rest) = (weight / n, weight % n);
    (0..n).map(move |i| share + Weight::from(i < rest))
}

/// A scope, held by its owner.
pub struct Scope {
    returned: Weight,
    items: Vec<Value>,
    pub then: Then,
}

/// What happens when a scope is over.
pub enum Then {
    /// The run's root scope: the run is over.
    Finish {
        outcome: RunOutcome,
        done: oneshot::Sender<RunOutcome>,
    },
    /// A fan-out scope: the branch that fanned out continues.
    Continue(Continuation),
}

/// The branch that opened a fan-out scope, suspended until the scope is over.
pub struct Continuation {
    /// The context it fanned out from.
    pub ctx: NodeId,
    /// Its weight in the enclosing scope.
    pub weight: Weight,
    /// The scopes enclosing it, innermost last.
    pub scopes: Arc<[ScopeId]>,
    /// The op that collects this scope, if its branches reach one.
    pub collect: Option<OpId>,
}

impl Scope {
    pub fn new(then: Then) -> Self {
        Self {
            returned: 0,
            items: Vec::new(),
            then,
        }
    }

    /// Takes back a branch's weight and collected item. Returns whether the scope is over.
    pub fn end(&mut self, weight: Weight, item: Option<Value>) -> bool {
        self.returned += weight;
        assert!(self.returned <= FULL, "scope returned more than its weight");
        self.items.extend(item);
        self.returned == FULL
    }

    pub fn into_parts(self) -> (Vec<Value>, Then) {
        (self.items, self.then)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shares_sum_to_the_whole() {
        for n in [1, 2, 3, 7, 1000] {
            let shares: Vec<_> = split(FULL, n).collect();
            assert_eq!(shares.len(), n);
            assert_eq!(shares.iter().sum::<Weight>(), FULL);
            assert!(shares.iter().all(|&s| s > 0));
        }
    }

    #[test]
    fn a_scope_ends_when_all_weight_is_back() {
        let (done, _) = oneshot::channel();
        let mut scope = Scope::new(Then::Finish {
            outcome: RunOutcome::default(),
            done,
        });
        let mut shares = split(FULL, 3);
        assert!(!scope.end(shares.next().unwrap(), Some(Value::from(1))));
        assert!(!scope.end(shares.next().unwrap(), None));
        assert!(scope.end(shares.next().unwrap(), Some(Value::from(3))));
        let (items, _) = scope.into_parts();
        assert_eq!(items, vec![Value::from(1), Value::from(3)]);
    }
}
