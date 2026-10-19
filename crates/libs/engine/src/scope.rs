//! Fan-out scopes and how they end: by counting.
//!
//! Every branch of a scope is created by the scope's owner: one per item when it fans out, or one
//! per item its stream source yields. So the owner knows how many branches the scope has: at once
//! for a fan-out, and when its source ends for a stream. Branches are numbered from 0, and the
//! scope is over once every index up to its count has ended. Branch ends travel straight to the
//! owner, so no message ordering between other workers can make a scope end early.
//!
//! Recovery can run a branch twice (design §7). The owner counts each index once: a second end of
//! the same branch is dropped.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Arc;

use namu_core::OpId;
use namu_core::value::Value;
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;

use crate::ids::{NodeId, ScopeId};
use crate::message::BranchId;
use crate::run::RunOutcome;
use crate::tree::Node;

/// A scope, held by its owner.
pub struct Scope {
    /// How many branches the scope has, once that is known.
    count: Option<u64>,
    ended: Indices,
    /// Collected items, by the index of the branch that brought them.
    items: BTreeMap<u64, Value>,
    pub then: Then,
    /// For a scope fed by a stream: one per item the source may still put in flight.
    pub credits: Option<Arc<Semaphore>>,
}

/// What happens when a scope is over.
pub enum Then {
    /// The run's root scope: the run is over.
    Finish,
    /// A fan-out scope: the branch that fanned out continues from `ctx`, which the scope holds
    /// until then.
    Continue {
        continuation: Continuation,
        ctx: Arc<Node>,
    },
}

/// The run's outcome as its owner collects it.
#[derive(Default)]
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
    /// The scopes enclosing it, innermost last.
    pub scopes: Arc<[ScopeId]>,
    /// The op that collects this scope, if its branches reach one.
    pub collect: Option<OpId>,
}

/// The stream task feeding a scope (design §6.5).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    /// The op that calls the stream task, in the continuation's context.
    pub call: OpId,
    /// The fan-out it feeds.
    pub fan_out: OpId,
    pub position: Position,
}

/// How far a stream has got.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Position {
    /// Items yielded so far; the next item is numbered this.
    pub yielded: u64,
    /// The last cursor the stream gave, and how many items had been yielded then.
    pub cursor: Option<(String, u64)>,
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
    /// A scope with `count` branches, if that is known yet.
    pub fn new(then: Then, count: Option<u64>) -> Self {
        Self {
            count,
            ended: Indices::default(),
            items: BTreeMap::new(),
            then,
            credits: None,
        }
    }

    pub fn with_credits(self, credits: Arc<Semaphore>) -> Self {
        Self {
            credits: Some(credits),
            ..self
        }
    }

    /// How many branches have ended.
    pub fn ended(&self) -> u64 {
        self.ended.len()
    }

    /// Branch `index` ended, with its collected item if it reached the collect.
    pub fn end(&mut self, index: u64, item: Option<Value>) -> Ended {
        if !self.ended.insert(index) {
            return Ended::Again;
        }
        if let Some(item) = item {
            self.items.insert(index, item);
        }
        self.state()
    }

    /// The scope has `count` branches: what its stream source learns when it ends.
    pub fn close(&mut self, count: u64) -> Ended {
        self.count = Some(count);
        self.state()
    }

    pub fn is_over(&self) -> bool {
        self.state() == Ended::Over
    }

    fn state(&self) -> Ended {
        match self.count {
            Some(count) if self.ended.len() == count => Ended::Over,
            _ => Ended::Open,
        }
    }

    /// The collected items, in branch order, and what happens next.
    pub fn into_parts(self) -> (Vec<Value>, Then) {
        (self.items.into_values().collect(), self.then)
    }
}

/// A set of indices that fill in from 0: every index below `low` is in, and `above` holds the
/// ones that ended ahead of an earlier one. Its size is how far ends arrive out of order, not how
/// many there were.
#[derive(Debug, Default)]
struct Indices {
    low: u64,
    above: BTreeSet<u64>,
}

impl Indices {
    /// Adds `index`; false if it was in already.
    fn insert(&mut self, index: u64) -> bool {
        if index < self.low || !self.above.insert(index) {
            return false;
        }
        while self.above.first() == Some(&self.low) {
            self.above.pop_first();
            self.low += 1;
        }
        true
    }

    fn len(&self) -> u64 {
        self.low + self.above.len() as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scope_is_over_when_every_branch_has_ended() {
        let mut scope = Scope::new(Then::Finish, Some(3));
        let ends = [
            scope.end(2, Some(Value::new(3))),
            scope.end(1, None),
            scope.end(0, Some(Value::new(1))),
        ];
        assert_eq!(ends, [Ended::Open, Ended::Open, Ended::Over]);
        let (items, _) = scope.into_parts();
        let items: Vec<i32> = items.into_iter().map(|item| item.take().unwrap()).collect();
        assert_eq!(items, [1, 3]);
    }

    #[test]
    fn a_branch_counts_once() {
        let mut scope = Scope::new(Then::Finish, Some(2));
        assert_eq!(scope.end(0, None), Ended::Open);
        assert_eq!(scope.end(0, None), Ended::Again);
        assert_eq!(scope.end(1, None), Ended::Over);
    }

    #[test]
    fn a_streamed_scope_is_over_once_closed_and_drained() {
        let mut scope = Scope::new(Then::Finish, None);
        assert_eq!(scope.end(0, None), Ended::Open);
        assert_eq!(scope.close(2), Ended::Open);
        assert_eq!(scope.end(1, None), Ended::Over);
        let mut empty = Scope::new(Then::Finish, None);
        assert_eq!(empty.close(0), Ended::Over);
    }

    #[test]
    fn indices_hold_only_what_arrived_out_of_order() {
        let mut ended = Indices::default();
        for index in [1, 3, 0, 2, 2, 5] {
            ended.insert(index);
        }
        assert_eq!((ended.low, ended.len()), (4, 5));
        assert_eq!(ended.above, BTreeSet::from([5]));
    }
}
