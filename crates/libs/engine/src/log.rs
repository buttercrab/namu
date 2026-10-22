//! The durable log: what a successor needs to take over a dead worker's work (design §7).
//!
//! Workers append records behind the critical path. An append is applied after every earlier
//! append from the same worker, and its [`Durable`] resolves once the records are durable; only
//! acknowledgements wait for that. Nothing reads the log on the common path: a successor reads a
//! dead worker's estate, and workers read nodes whose holder is dead.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use namu_core::value::Value;
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

use crate::ids::{NodeId, RunId, ScopeId, WorkerId, owner};
use crate::message::{Branch, BranchId, Report};
use crate::run::RunInfo;
use crate::scope::{Continuation, Feed, Position, Source};
use crate::tree::Node;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Record {
    /// A run, written by its owner when it starts.
    Run(Arc<RunInfo>),
    /// Nodes that a record next to them refers to.
    Nodes {
        run: RunId,
        nodes: Vec<Arc<Node>>,
    },
    /// A branch as it stops at `holder`.
    Branch {
        run: RunId,
        holder: WorkerId,
        branch: Branch,
    },
    /// The branch has left `holder` (or ended there).
    BranchGone {
        run: RunId,
        holder: WorkerId,
        id: BranchId,
        hop: u64,
    },
    /// A scope opened by its owner: `continuation` is `None` for the run's root scope, `source`
    /// is the stream feeding it, if one does, `count` its number of branches if known, and
    /// `feeds` the scope it is windowed into, if it is.
    Scope {
        run: RunId,
        scope: ScopeId,
        continuation: Option<Continuation>,
        source: Option<Source>,
        count: Option<u64>,
        feeds: Option<(ScopeId, Feed)>,
    },
    /// How far windowing `scope` into `into` has got.
    Fed {
        run: RunId,
        scope: ScopeId,
        into: ScopeId,
        feed: Feed,
    },
    /// How far the stream feeding `scope` has got.
    Position {
        run: RunId,
        scope: ScopeId,
        position: Position,
    },
    /// The stream feeding `scope` has ended: the scope has `count` branches.
    Closed {
        run: RunId,
        scope: ScopeId,
        count: u64,
    },
    /// The scope is over and its continuation recorded.
    ScopeGone {
        run: RunId,
        scope: ScopeId,
    },
    End {
        run: RunId,
        scope: ScopeId,
        end: EndRecord,
    },
    /// The fold of `scope` is done through branch `through` (exclusive): `accumulator` is its
    /// value there. It replaces the scope's previous one.
    Folded {
        run: RunId,
        scope: ScopeId,
        through: u64,
        accumulator: Value,
    },
    Report {
        run: RunId,
        report: ReportRecord,
    },
    /// The run is over: everything about it can go.
    RunGone {
        run: RunId,
        root_scope: ScopeId,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndRecord {
    /// The branch's index in its scope.
    pub index: u64,
    pub item: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportRecord {
    pub branch: BranchId,
    pub report: Report,
}

/// The nodes of a run's log that nothing live reaches: neither the context of a live record (a
/// branch's, or a scope's continuation) nor an ancestor of one. `parents` maps every logged node
/// to its parent. A log drops them, so it holds the live frontier of a run rather than its history
/// (design §7.5). No dead node comes back: a branch that could still reach one is recorded
/// somewhere, and its record's chain is live.
pub fn dead_nodes(
    parents: &HashMap<NodeId, Option<NodeId>>,
    live: impl IntoIterator<Item = NodeId>,
) -> Vec<NodeId> {
    let mut reached = HashSet::new();
    for ctx in live {
        let mut cursor = Some(ctx);
        while let Some(id) = cursor {
            if !reached.insert(id) {
                break;
            }
            cursor = parents.get(&id).copied().flatten();
        }
    }
    parents
        .keys()
        .filter(|id| !reached.contains(id))
        .copied()
        .collect()
}

/// A log sweeps a run's dead nodes when it holds at least this many, and twice as many as after
/// its last sweep: a sweep costs O(nodes), so this is O(1) per node logged.
pub const SWEEP_FLOOR: usize = 64;

/// Resolves once an append is durable. Dropped if the append failed.
pub type Durable = oneshot::Receiver<()>;

/// What one worker owned and held in one run.
#[derive(Debug, Clone, Default)]
pub struct RunEstate {
    pub info: Option<Arc<RunInfo>>,
    /// Reports, if the worker owned the run.
    pub reports: Vec<ReportRecord>,
    /// Scopes it opened, with their ends.
    pub scopes: Vec<ScopeEstate>,
    /// Branches it held.
    pub branches: Vec<Branch>,
}

#[derive(Debug, Clone)]
pub struct ScopeEstate {
    pub scope: ScopeId,
    pub continuation: Option<Continuation>,
    /// The stream feeding the scope, at its last logged position.
    pub source: Option<Source>,
    /// How many branches the scope has, if known.
    pub count: Option<u64>,
    pub ends: Vec<EndRecord>,
    /// How far its fold has got, if it folds: through which branch, and the accumulator there.
    pub folded: Option<(u64, Value)>,
    /// The scope it is windowed into, and how far that has got, if it is.
    pub feeds: Option<(ScopeId, Feed)>,
}

#[async_trait]
pub trait Log: Send + Sync + 'static {
    /// Appends `records`, applied after every earlier append from this worker.
    fn append(&self, records: Vec<Record>) -> Durable;

    /// Appends `records` without waiting for them to be durable.
    fn record(&self, records: Vec<Record>) {
        drop(self.append(records));
    }

    async fn node(&self, run: RunId, id: NodeId) -> anyhow::Result<Option<Arc<Node>>>;

    async fn run(&self, run: RunId) -> anyhow::Result<Option<Arc<RunInfo>>>;

    /// Everything `worker` owned or held, per run, for its successor.
    async fn estate(&self, worker: WorkerId) -> anyhow::Result<Vec<RunEstate>>;
}

/// A log in this process's memory: durable for as long as the process lives. It is what
/// `LocalCluster` uses, which makes worker crashes testable in one process.
#[derive(Debug, Default)]
pub struct MemoryLog {
    runs: Mutex<HashMap<RunId, MemoryRun>>,
    /// The most nodes any run held at once.
    peak: std::sync::atomic::AtomicUsize,
}

#[derive(Debug, Default)]
struct MemoryRun {
    info: Option<Arc<RunInfo>>,
    nodes: HashMap<NodeId, Arc<Node>>,
    branches: HashMap<(WorkerId, BranchId, u64), Branch>,
    scopes: HashMap<ScopeId, ScopeEstate>,
    reports: Vec<ReportRecord>,
    /// Nodes held after the last sweep.
    swept: usize,
}

impl MemoryRun {
    /// Drops the nodes nothing live reaches, if enough have been logged since the last sweep.
    fn sweep(&mut self) {
        if self.nodes.len() < SWEEP_FLOOR.max(2 * self.swept) {
            return;
        }
        let parents = self
            .nodes
            .iter()
            .map(|(&id, node)| (id, node.parent_id()))
            .collect();
        let live = self.branches.values().map(|branch| branch.ctx).chain(
            self.scopes
                .values()
                .filter_map(|scope| scope.continuation.as_ref().map(|c| c.ctx)),
        );
        for id in dead_nodes(&parents, live) {
            self.nodes.remove(&id);
        }
        self.swept = self.nodes.len();
    }
}

impl MemoryLog {
    /// The most nodes any run's log held at once.
    pub fn peak_nodes(&self) -> usize {
        self.peak.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn apply(&self, records: Vec<Record>) {
        let mut runs = self.runs.lock().unwrap();
        for record in records {
            match record {
                Record::Run(info) => {
                    let id = info.id;
                    runs.entry(id).or_default().info = Some(info);
                }
                Record::Nodes { run, nodes } => {
                    let held = &mut runs.entry(run).or_default().nodes;
                    held.extend(nodes.into_iter().map(|node| (node.id(), node)));
                    let held = held.len();
                    self.peak
                        .fetch_max(held, std::sync::atomic::Ordering::Relaxed);
                }
                Record::Branch {
                    run,
                    holder,
                    branch,
                } => {
                    let key = (holder, branch.id, branch.hop);
                    runs.entry(run).or_default().branches.insert(key, branch);
                }
                Record::BranchGone {
                    run,
                    holder,
                    id,
                    hop,
                } => {
                    if let Some(run) = runs.get_mut(&run) {
                        run.branches.remove(&(holder, id, hop));
                        run.sweep();
                    }
                }
                Record::Scope {
                    run,
                    scope,
                    continuation,
                    source,
                    count,
                    feeds,
                } => {
                    let estate = ScopeEstate {
                        scope,
                        continuation,
                        source,
                        count,
                        ends: Vec::new(),
                        folded: None,
                        feeds,
                    };
                    runs.entry(run).or_default().scopes.insert(scope, estate);
                }
                Record::Position {
                    run,
                    scope,
                    position,
                } => {
                    let source = runs
                        .get_mut(&run)
                        .and_then(|run| run.scopes.get_mut(&scope))
                        .and_then(|scope| scope.source.as_mut());
                    if let Some(source) = source {
                        source.position = position;
                    }
                }
                Record::Closed { run, scope, count } => {
                    let scope = runs
                        .get_mut(&run)
                        .and_then(|run| run.scopes.get_mut(&scope));
                    if let Some(scope) = scope {
                        scope.count = Some(count);
                    }
                }
                Record::ScopeGone { run, scope } => {
                    if let Some(run) = runs.get_mut(&run) {
                        run.scopes.remove(&scope);
                        run.sweep();
                    }
                }
                Record::End { run, scope, end } => {
                    let run = runs.entry(run).or_default();
                    if let Some(scope) = run.scopes.get_mut(&scope) {
                        scope.ends.push(end);
                    }
                }
                Record::Fed {
                    run,
                    scope,
                    into,
                    feed,
                } => {
                    let scope = runs
                        .get_mut(&run)
                        .and_then(|run| run.scopes.get_mut(&scope));
                    if let Some(scope) = scope {
                        scope.feeds = Some((into, feed));
                    }
                }
                Record::Folded {
                    run,
                    scope,
                    through,
                    accumulator,
                } => {
                    let scope = runs
                        .get_mut(&run)
                        .and_then(|run| run.scopes.get_mut(&scope));
                    if let Some(scope) = scope {
                        scope.folded = Some((through, accumulator));
                    }
                }
                Record::Report { run, report } => {
                    runs.entry(run).or_default().reports.push(report);
                }
                Record::RunGone { run, .. } => {
                    runs.remove(&run);
                }
            }
        }
    }
}

#[async_trait]
impl Log for MemoryLog {
    fn append(&self, records: Vec<Record>) -> Durable {
        self.apply(records);
        let (durable, done) = oneshot::channel();
        let _ = durable.send(());
        done
    }

    async fn node(&self, run: RunId, id: NodeId) -> anyhow::Result<Option<Arc<Node>>> {
        let runs = self.runs.lock().unwrap();
        Ok(runs.get(&run).and_then(|run| run.nodes.get(&id).cloned()))
    }

    async fn run(&self, run: RunId) -> anyhow::Result<Option<Arc<RunInfo>>> {
        let runs = self.runs.lock().unwrap();
        Ok(runs.get(&run).and_then(|run| run.info.clone()))
    }

    async fn estate(&self, worker: WorkerId) -> anyhow::Result<Vec<RunEstate>> {
        let runs = self.runs.lock().unwrap();
        let estates = runs.values().map(|run| {
            let owns_run = run.info.as_ref().is_some_and(|info| info.owner() == worker);
            RunEstate {
                info: run.info.clone(),
                reports: match owns_run {
                    true => run.reports.clone(),
                    false => Vec::new(),
                },
                scopes: run
                    .scopes
                    .values()
                    .filter(|scope| owner(scope.scope) == worker)
                    .cloned()
                    .collect(),
                branches: run
                    .branches
                    .iter()
                    .filter(|((holder, _, _), _)| *holder == worker)
                    .map(|(_, branch)| branch.clone())
                    .collect(),
            }
        });
        Ok(estates
            .filter(|estate| {
                !estate.reports.is_empty()
                    || !estate.scopes.is_empty()
                    || !estate.branches.is_empty()
            })
            .collect())
    }
}
