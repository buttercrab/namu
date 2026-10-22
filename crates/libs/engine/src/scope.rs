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
//!
//! A scope closed by a reduce folds its items instead of collecting them: in branch order, as the
//! ends come in, one fold at a time. Only items that ended ahead of an earlier one wait in memory;
//! the scope is over once every branch has ended and every item is folded.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Arc;

use namu_core::OpId;
use namu_core::value::Value;
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;

use crate::ids::{Lineage, NodeId, ScopeId};
use crate::message::BranchId;
use crate::run::RunOutcome;
use crate::tasks::{CallKey, TaskRef};
use crate::tree::Node;

/// A scope, held by its owner.
pub struct Scope {
    /// How many branches the scope has, once that is known.
    count: Option<u64>,
    ended: Indices,
    /// Collected items, by the index of the branch that brought them; for a fold, those not yet
    /// folded.
    items: BTreeMap<u64, Value>,
    pub fold: Option<Fold>,
    pub feed: Option<Feed>,
    pub then: Then,
    /// For a scope fed by a stream: one per item the source may still put in flight.
    pub credits: Option<Arc<Semaphore>>,
}

/// A scope's fold: `task(accumulator, item)` over its items in branch order.
pub struct Fold {
    pub task: TaskRef,
    /// The reducing collect.
    pub op: OpId,
    pub accumulator: Value,
    /// The next branch to fold: every one before it is folded (or brought no item).
    pub next: u64,
    /// A fold step is running.
    pub busy: bool,
}

/// A scope whose items go out in windows: every `size` items, in branch order, make a branch of
/// the scope it feeds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Feed {
    pub size: u64,
    /// The window op.
    pub op: OpId,
    /// The first branch of the window being filled: every item before it has gone out.
    pub start: u64,
    /// Windows gone out so far; the next one is numbered this.
    pub windows: u64,
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
    /// A scope windowed into scope `into`, which knows how many windows it has once this one is
    /// over. Like a fan-out scope, it holds the context it was opened in (and the windows go out
    /// from it), but the branch that opened it continues from the windows' scope.
    Feed {
        into: ScopeId,
        continuation: Continuation,
        ctx: Arc<Node>,
    },
}

impl Then {
    /// How a scope that is not the run's root was opened: by whom, and in what context.
    pub fn opened(&self) -> Option<(&Continuation, &Arc<Node>)> {
        match self {
            Then::Finish => None,
            Then::Continue { continuation, ctx }
            | Then::Feed {
                continuation, ctx, ..
            } => Some((continuation, ctx)),
        }
    }
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
    pub lineage: Lineage,
    /// Its clock once it fanned out.
    pub clock: u64,
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
    /// The call's key: the stream has it however often it is opened.
    pub key: CallKey,
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
            fold: None,
            feed: None,
            then,
            credits: None,
        }
    }

    /// With `fold` and `feed`, if given.
    pub fn with(self, fold: Option<Fold>, feed: Option<Feed>) -> Self {
        let scope = match fold {
            Some(fold) => self.with_fold(fold),
            None => self,
        };
        match feed {
            Some(feed) => scope.with_feed(feed),
            None => scope,
        }
    }

    /// The scope sends its items out in windows. Items already gone out (below `feed.start`) are
    /// dropped.
    pub fn with_feed(mut self, feed: Feed) -> Self {
        self.items = self.items.split_off(&feed.start);
        self.feed = Some(feed);
        self
    }

    /// The windows ready to go out, each with the feed as it is once it has: each full one, in
    /// branch order, and the last, partial one once every branch has ended. Their items leave the
    /// scope.
    pub fn next_windows(&mut self) -> Vec<(Feed, Vec<Value>)> {
        let Some(feed) = self.feed.as_mut() else {
            return Vec::new();
        };
        let mut windows = Vec::new();
        loop {
            // The window from `start`: its items, as far as the branches have ended in order.
            let mut index = feed.start;
            let mut taken = Vec::new();
            while (taken.len() as u64) < feed.size && self.ended.contains(index) {
                if self.items.contains_key(&index) {
                    taken.push(index);
                }
                index += 1;
            }
            let full = taken.len() as u64 == feed.size;
            let last = !taken.is_empty() && self.count == Some(index) && self.ended.len() == index;
            if !full && !last {
                break;
            }
            let items = taken
                .iter()
                .map(|index| self.items.remove(index).expect("taken above"))
                .collect();
            feed.start = index;
            feed.windows += 1;
            windows.push((*feed, items));
        }
        windows
    }

    /// The scope folds its items with `fold` instead of collecting them. Items already folded
    /// (below `fold.next`) are dropped.
    pub fn with_fold(mut self, fold: Fold) -> Self {
        self.items = self.items.split_off(&fold.next);
        self.fold = Some(fold);
        self
    }

    /// The next item to fold, and its index, if it has ended and no fold step is running. Ended
    /// branches that brought no item are passed over.
    pub fn next_fold(&mut self) -> Option<(u64, Value)> {
        let fold = self.fold.as_mut()?;
        if fold.busy {
            return None;
        }
        while self.ended.contains(fold.next) {
            match self.items.remove(&fold.next) {
                Some(item) => {
                    fold.busy = true;
                    return Some((fold.next, item));
                }
                None => fold.next += 1,
            }
        }
        None
    }

    /// The running fold step is done: `accumulator` is the fold through its item.
    pub fn folded(&mut self, accumulator: Value) {
        let fold = self.fold.as_mut().expect("a fold step ran");
        fold.accumulator = accumulator;
        fold.next += 1;
        fold.busy = false;
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
        let folded = |count| self.fold.as_ref().is_none_or(|fold| fold.next == count);
        // A windowed scope is over once its last window has gone out.
        let fed = || self.feed.is_none() || self.items.is_empty();
        match self.count {
            Some(count) if self.ended.len() == count && folded(count) && fed() => Ended::Over,
            _ => Ended::Open,
        }
    }

    /// What the scope gathered (its items as a list, in branch order, or its fold's last
    /// accumulator), and what happens next.
    pub fn into_parts(self) -> (Value, Then) {
        let gathered = match self.fold {
            Some(fold) => fold.accumulator,
            None => Value::list(self.items.into_values().collect()),
        };
        (gathered, self.then)
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
    fn contains(&self, index: u64) -> bool {
        index < self.low || self.above.contains(&index)
    }

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
        assert_eq!(items.take::<Vec<i32>>().unwrap(), [1, 3]);
    }

    fn feeding() -> Then {
        let continuation = Continuation {
            branch: BranchId { scope: 0, index: 0 },
            hop: 0,
            lineage: 0,
            clock: 1,
            ctx: 0,
            scopes: Arc::from([0]),
            collect: Some(9),
        };
        let ctx = Arc::new(Node::root(0));
        Then::Feed {
            into: 1,
            continuation,
            ctx,
        }
    }

    #[test]
    fn windows_go_out_in_branch_order_and_the_last_one_partial() {
        let feed = Feed {
            size: 2,
            op: 9,
            start: 0,
            windows: 0,
        };
        let mut scope = Scope::new(feeding(), Some(5)).with_feed(feed);
        let ints = |items: Vec<Value>| -> Vec<i32> {
            items.into_iter().map(|item| item.take().unwrap()).collect()
        };
        scope.end(1, Some(Value::new(1)));
        assert!(scope.next_windows().is_empty(), "0 has not ended");
        scope.end(0, Some(Value::new(0)));
        let ready = scope.next_windows();
        assert_eq!(ready.len(), 1);
        assert_eq!((ready[0].0.windows, ready[0].0.start), (1, 2));
        assert_eq!(ints(ready[0].1.clone()), [0, 1]);
        // 2 was dropped: the next window takes 3 and 4.
        scope.end(2, None);
        scope.end(4, Some(Value::new(4)));
        assert!(scope.next_windows().is_empty());
        scope.end(3, Some(Value::new(3)));
        let ready = scope.next_windows();
        assert_eq!(ints(ready[0].1.clone()), [3, 4]);
        assert!(scope.is_over());

        // A last window short of the size goes out once every branch has ended.
        let mut scope = Scope::new(feeding(), Some(3)).with_feed(feed);
        for index in 0..3 {
            scope.end(index, Some(Value::new(index as i32)));
        }
        let ready = scope.next_windows();
        let windows: Vec<Vec<i32>> = ready.into_iter().map(|(_, items)| ints(items)).collect();
        assert_eq!(windows, [vec![0, 1], vec![2]]);
        assert!(scope.is_over());
    }

    #[test]
    fn a_fold_goes_in_branch_order_whatever_order_branches_end_in() {
        let fold = Fold {
            task: TaskRef {
                id: "merge".into(),
                version: String::new(),
            },
            op: 7,
            accumulator: Value::new(0),
            next: 0,
            busy: false,
        };
        let mut scope = Scope::new(Then::Finish, Some(4)).with_fold(fold);
        // 2 ends first: nothing to fold until 0 has.
        assert_eq!(scope.end(2, Some(Value::new(20))), Ended::Open);
        assert!(scope.next_fold().is_none());
        assert_eq!(scope.end(0, Some(Value::new(1))), Ended::Open);
        let (index, item) = scope.next_fold().unwrap();
        assert_eq!((index, item.take::<i32>().unwrap()), (0, 1));
        // One step at a time.
        assert!(scope.next_fold().is_none());
        scope.folded(Value::new(1));
        // 1 ended with no item (it was dropped): passed over, to 2.
        assert_eq!(scope.end(1, None), Ended::Open);
        let (index, _) = scope.next_fold().unwrap();
        assert_eq!(index, 2);
        scope.folded(Value::new(21));
        assert_eq!(scope.end(3, None), Ended::Open);
        // Every branch has ended, and passing over 3 finishes the fold.
        assert!(scope.next_fold().is_none());
        assert!(scope.is_over());
        let (accumulator, _) = scope.into_parts();
        assert_eq!(accumulator.take::<i32>().unwrap(), 21);
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
