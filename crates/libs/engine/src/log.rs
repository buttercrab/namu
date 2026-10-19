//! The durable log: what a successor needs to take over a dead worker's work (design §7).
//!
//! Workers append records behind the critical path. An append is applied after every earlier
//! append from the same worker, and its [`Durable`] resolves once the records are durable; only
//! acknowledgements wait for that. Nothing reads the log on the common path: a successor reads a
//! dead worker's estate, and workers read nodes whose holder is dead.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::oneshot;

use crate::ids::{NodeId, RunId, ScopeId, WorkerId, owner};
use crate::message::{Branch, BranchId, Report};
use crate::run::RunInfo;
use crate::scope::{Continuation, Position, Source};
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
    /// is the stream feeding it, if one does, and `count` its number of branches if known.
    Scope {
        run: RunId,
        scope: ScopeId,
        continuation: Option<Continuation>,
        source: Option<Source>,
        count: Option<u64>,
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
    Report {
        run: RunId,
        report: ReportRecord,
    },
    /// The run is over: everything about it can go.
    RunGone(RunId),
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
}

#[derive(Debug, Default)]
struct MemoryRun {
    info: Option<Arc<RunInfo>>,
    nodes: HashMap<NodeId, Arc<Node>>,
    branches: HashMap<(WorkerId, BranchId, u64), Branch>,
    scopes: HashMap<ScopeId, ScopeEstate>,
    reports: Vec<ReportRecord>,
}

impl MemoryLog {
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
                    }
                }
                Record::Scope {
                    run,
                    scope,
                    continuation,
                    source,
                    count,
                } => {
                    let estate = ScopeEstate {
                        scope,
                        continuation,
                        source,
                        count,
                        ends: Vec::new(),
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
                    }
                }
                Record::End { run, scope, end } => {
                    let run = runs.entry(run).or_default();
                    if let Some(scope) = run.scopes.get_mut(&scope) {
                        scope.ends.push(end);
                    }
                }
                Record::Report { run, report } => {
                    runs.entry(run).or_default().reports.push(report);
                }
                Record::RunGone(run) => {
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
