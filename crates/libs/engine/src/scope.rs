//! Fan-out scopes and how they end: weight throwing.
//!
//! A scope starts with [`FULL`] weight, split among its branches when they are created. Weight is
//! conserved: a branch passes its weight along with its work, and returns it to the scope's owner
//! when it ends, together with its collected item if it reached the collect. The scope is over
//! once the owner holds the full weight again. Creating branches never needs a message to the
//! owner, so no message ordering between workers can make a scope end early.
//!
//! Recovery can run a branch twice (design §7). The owner counts each branch once: a second end of
//! the same branch is dropped.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use namu_core::OpId;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ids::{NodeId, ScopeId};
use crate::message::BranchId;
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
    /// Collected items, by the branch that brought them.
    items: HashMap<BranchId, Value>,
    ended: HashSet<BranchId>,
    pub then: Then,
}

/// What happens when a scope is over.
pub enum Then {
    /// The run's root scope: the run is over.
    Finish(Box<Finish>),
    /// A fan-out scope: the branch that fanned out continues.
    Continue(Continuation),
}

/// The run's outcome as its owner collects it.
pub struct Finish {
    pub outcome: RunOutcome,
    /// Branches whose report is recorded.
    pub reported: HashSet<BranchId>,
}

/// The branch that opened a fan-out scope, suspended until the scope is over.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Continuation {
    pub branch: BranchId,
    /// Its hop when it fanned out; it continues as the next hop.
    pub hop: u64,
    /// The context it fanned out from.
    pub ctx: NodeId,
    /// Its weight in the enclosing scope.
    pub weight: Weight,
    /// The scopes enclosing it, innermost last.
    pub scopes: Arc<[ScopeId]>,
    /// The op that collects this scope, if its branches reach one.
    pub collect: Option<OpId>,
}

/// How a branch's end changed its scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ended {
    /// The branch had ended before; nothing changed.
    Again,
    Open,
    Over,
}

impl Scope {
    pub fn new(then: Then) -> Self {
        Self {
            returned: 0,
            items: HashMap::new(),
            ended: HashSet::new(),
            then,
        }
    }

    /// Takes back a branch's weight and collected item.
    pub fn end(&mut self, branch: BranchId, weight: Weight, item: Option<Value>) -> Ended {
        if !self.ended.insert(branch) {
            return Ended::Again;
        }
        self.returned += weight;
        assert!(self.returned <= FULL, "scope returned more than its weight");
        if let Some(item) = item {
            self.items.insert(branch, item);
        }
        match self.returned == FULL {
            true => Ended::Over,
            false => Ended::Open,
        }
    }

    pub fn has_ended(&self, branch: BranchId) -> bool {
        self.ended.contains(&branch)
    }

    /// The collected items, in branch order, and what happens next.
    pub fn into_parts(self) -> (Vec<Value>, Then) {
        let mut items: Vec<_> = self.items.into_iter().collect();
        items.sort_unstable_by_key(|(branch, _)| *branch);
        (items.into_iter().map(|(_, item)| item).collect(), self.then)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn branch(index: u64) -> BranchId {
        BranchId { scope: 1, index }
    }

    fn finish() -> Then {
        Then::Finish(Box::new(Finish {
            outcome: RunOutcome::default(),
            reported: HashSet::new(),
        }))
    }

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
        let mut scope = Scope::new(finish());
        let mut shares = split(FULL, 3);
        let ends = [
            scope.end(branch(2), shares.next().unwrap(), Some(Value::from(3))),
            scope.end(branch(1), shares.next().unwrap(), None),
            scope.end(branch(0), shares.next().unwrap(), Some(Value::from(1))),
        ];
        assert_eq!(ends, [Ended::Open, Ended::Open, Ended::Over]);
        let (items, _) = scope.into_parts();
        assert_eq!(items, vec![Value::from(1), Value::from(3)]);
    }

    #[test]
    fn a_branch_counts_once() {
        let mut scope = Scope::new(finish());
        let mut shares = split(FULL, 2);
        let share = shares.next().unwrap();
        assert_eq!(scope.end(branch(0), share, None), Ended::Open);
        assert_eq!(scope.end(branch(0), share, None), Ended::Again);
        assert_eq!(
            scope.end(branch(1), shares.next().unwrap(), None),
            Ended::Over
        );
    }
}
