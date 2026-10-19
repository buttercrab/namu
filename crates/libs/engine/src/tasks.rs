//! The tasks a worker can run.
//!
//! A task takes its arguments as values and returns one value; a stream task yields its outputs
//! one at a time; a batch task runs as a session that takes items in and gives results back in
//! steps. A task with several outputs returns a tuple. Runtimes outside this process see the
//! arguments packed by the task ABI's convention ([`encode_args`](namu_core::value::encode_args)).

use std::cell::Cell;
use std::collections::HashMap;
use std::fmt::Display;
use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use namu_core::OpId;
use namu_core::value::{Data, Value, from_args};
use serde::{Deserialize, Serialize};

/// A task call's arguments, in order.
pub type Args = Vec<Value>;

/// Names one task call, for tasks with side effects to deduplicate on (design §7.5): the same
/// each time recovery runs the call again, and different for every other call. It is the call's
/// place in its run: the `call`-th call of its branch, at `op`. A branch run again from a record
/// starts from the count recorded with it, so it names its calls as it did the first time; the
/// op tells calls apart if a task's output sent the branch down another path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CallKey {
    pub run: RunId,
    pub branch: BranchId,
    pub call: u64,
    pub op: OpId,
}

impl Display for CallKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self {
            run,
            branch,
            call,
            op,
        } = self;
        write!(f, "{run}/{:x}.{}/{call}/{op}", branch.scope, branch.index)
    }
}

thread_local! {
    static CALL_KEY: Cell<Option<CallKey>> = const { Cell::new(None) };
}

/// The key of the call an in-process task is running for, read from inside the task. A stream has
/// the key of the call that opened it, while it opens and while each item is pulled. `None`
/// outside a call, and in batch tasks, whose steps serve many calls at once.
pub fn call_key() -> Option<CallKey> {
    CALL_KEY.get()
}

/// Runs `f` with `key` as the current call's key.
fn with_key<T>(key: CallKey, f: impl FnOnce() -> T) -> T {
    struct Reset(Option<CallKey>);
    impl Drop for Reset {
        fn drop(&mut self) {
            CALL_KEY.set(self.0);
        }
    }
    let _reset = Reset(CALL_KEY.replace(Some(key)));
    f()
}

use crate::ids::RunId;
use crate::message::BranchId;
use crate::run::RunSpec;

/// A task at the version a run uses.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TaskRef {
    pub id: String,
    /// Empty for in-process tasks, which are not versioned.
    pub version: String,
}

/// The tasks one worker serves.
#[async_trait]
pub trait Tasks: Send + Sync + 'static {
    /// Called before any branch of `run` runs on this worker, e.g. to load the tasks it uses.
    async fn prepare(&self, _run: &RunSpec) -> anyhow::Result<()> {
        Ok(())
    }

    /// How this worker runs `task`, or `None` if it does not serve it.
    fn task(&self, task: &TaskRef) -> Option<Task>;
}

/// How a worker runs a task.
#[derive(Clone)]
pub struct Task {
    pub kind: Kind,
    /// Whether the task has external side effects that must not be repeated by recovery (design
    /// §7.5). The branch is recorded past such a task before it continues.
    pub checkpoint: bool,
}

#[derive(Clone)]
pub enum Kind {
    /// Runs once per call.
    Call(Arc<dyn CallTask>),
    /// Yields items over time (design §6.5).
    Stream(Stream),
    /// Runs in sessions that take items in and give results back in steps (design §6.6).
    Batch(Batch),
}

#[derive(Clone)]
pub struct Stream {
    pub task: Arc<dyn StreamTask>,
    /// How many of its items may be in flight at once.
    pub window: usize,
}

#[derive(Clone)]
pub struct Batch {
    pub task: Arc<dyn BatchTask>,
    pub policy: BatchPolicy,
}

impl Task {
    pub fn new(kind: Kind) -> Self {
        Self {
            kind,
            checkpoint: false,
        }
    }
}

/// Errors, for every kind of task: `Ok(Err(message))` is the task's own failure, which fails its
/// branch; `Err` is a failure of the machinery around it.
pub trait CallTask: Send + Sync {
    /// Runs the call named `key` on `args`.
    fn call(&self, args: Args, key: CallKey) -> anyhow::Result<Result<Value, String>>;
}

pub trait StreamTask: Send + Sync {
    /// Opens the stream for the call named `key`, at `cursor` if one of its items gave one. The
    /// open stream keeps its task alive.
    fn open(
        self: Arc<Self>,
        args: Args,
        cursor: Option<String>,
        key: CallKey,
    ) -> anyhow::Result<Result<Box<dyn TaskStream>, String>>;
}

pub trait BatchTask: Send + Sync {
    /// Opens a session. The session keeps its task alive.
    fn open(self: Arc<Self>) -> anyhow::Result<Result<Box<dyn BatchSession>, String>>;
}

/// Items of a stream in flight at once, unless the task says otherwise.
pub const DEFAULT_WINDOW: usize = 1024;

/// An open stream task.
pub trait TaskStream: Send {
    /// The next item, `None` once the stream is over. `Ok(Err(message))` is the task's own
    /// failure, which ends the stream; `Err` is a failure of the machinery around it.
    fn next(&mut self) -> anyhow::Result<Option<Result<StreamItem, String>>>;
}

pub struct StreamItem {
    pub value: Value,
    /// Where the stream can be reopened to continue after this item, if it can.
    pub cursor: Option<String>,
}

/// A batch task's session (design §6.6): a long-lived instance that takes items in and gives
/// results back, step by step.
pub trait BatchSession: Send {
    /// Hands the session `admit` and returns the items it finished, in any order: all of them for
    /// a dynamic batch, possibly fewer (and possibly earlier ones) for a continuous one. `Err` is a
    /// failure of the whole session, which fails every item in it.
    fn step(&mut self, admit: Vec<(u64, Args)>) -> anyhow::Result<Finished>;
}

/// The items a batch step finished: each item's id, and its output or its own error.
pub type Finished = Vec<(u64, Result<Value, String>)>;

/// How a batch task's sessions are run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatchPolicy {
    /// Items in a session at once.
    pub size: usize,
    /// If set, an idle session waits for `size` items, or this long for the oldest, before it
    /// steps: fuller batches for some latency.
    pub max_wait: Option<Duration>,
    /// Sessions per worker, e.g. one per GPU.
    pub slots: usize,
}

impl Default for BatchPolicy {
    fn default() -> Self {
        Self {
            size: 32,
            max_wait: None,
            slots: 1,
        }
    }
}

/// A batch task that keeps items across steps (continuous batching), written in Rust.
pub trait Continuous: Send + 'static {
    type In: Data;
    type Out: Data;

    /// Takes in `admit` and returns what finished in this step.
    fn step(&mut self, admit: Vec<(u64, Self::In)>) -> Vec<(u64, Result<Self::Out, String>)>;
}

/// Unpacks a task's output into its `arity` outputs: a task with several returns a tuple.
pub fn unpack_output(output: Value, arity: usize) -> anyhow::Result<Vec<Value>> {
    match arity {
        0 => Ok(Vec::new()),
        1 => Ok(vec![output]),
        n => {
            let parts = output.items()?;
            anyhow::ensure!(
                parts.len() == n,
                "expected {n} outputs, got {}",
                parts.len()
            );
            Ok(parts)
        }
    }
}

/// Tasks implemented as Rust functions in this process: the fastest runtime, with no
/// encoding at all: values pass between them as they are. They are not versioned.
#[derive(Default)]
pub struct FnTasks {
    tasks: HashMap<String, Task>,
}

impl FnTasks {
    pub fn new() -> Self {
        Self::default()
    }

    fn register(mut self, task: &str, kind: Kind) -> Self {
        self.tasks.insert(task.to_string(), Task::new(kind));
        self
    }

    fn registered(&mut self, task: &str) -> &mut Task {
        self.tasks
            .get_mut(task)
            .unwrap_or_else(|| panic!("task {task} is not registered"))
    }

    /// Registers `task`. `I` is the input: `()` for no arguments, the argument itself for one, a
    /// tuple for several.
    pub fn with<I, O, E>(
        self,
        task: &str,
        f: impl Fn(I) -> Result<O, E> + Send + Sync + 'static,
    ) -> Self
    where
        I: Data,
        O: Data,
        E: Display,
    {
        let call = move |args: Args| {
            let input = from_args(args).map_err(|err| format!("bad input: {err:#}"))?;
            f(input).map(Value::new).map_err(|err| err.to_string())
        };
        self.register(task, Kind::Call(Arc::new(FnCall(call))))
    }

    /// Registers a stream task: `f` returns the items, which are produced as they are pulled. The
    /// items give no cursors, so a resumed stream is opened again and skipped forward.
    pub fn with_stream<I, O, E, S>(
        self,
        task: &str,
        f: impl Fn(I) -> Result<S, E> + Send + Sync + 'static,
    ) -> Self
    where
        I: Data,
        O: Data,
        E: Display,
        S: Iterator<Item = Result<O, E>> + Send + 'static,
    {
        let open = move |args: Args| -> Result<Items, String> {
            let input = from_args(args).map_err(|err| format!("bad input: {err:#}"))?;
            let items = f(input).map_err(|err| err.to_string())?;
            Ok(Box::new(items.map(|item| {
                item.map(Value::new).map_err(|err| err.to_string())
            })))
        };
        let kind = Kind::Stream(Stream {
            task: Arc::new(FnStreamTask(open)),
            window: DEFAULT_WINDOW,
        });
        self.register(task, kind)
    }

    /// Registers a batch task run by dynamic batching: each step calls `f` on the items given.
    pub fn with_batch<I, O, E>(
        self,
        task: &str,
        f: impl Fn(Vec<I>) -> Result<Vec<Result<O, E>>, E> + Send + Sync + 'static,
    ) -> Self
    where
        I: Data,
        O: Data,
        E: Display + 'static,
    {
        let f = Arc::new(f);
        let open = move || -> Box<dyn BatchSession> {
            let f = f.clone();
            Box::new(Dynamic {
                f: move |items: Vec<I>| f(items),
                types: PhantomData,
            })
        };
        self.with_sessions(task, open)
    }

    /// Registers a batch task run by continuous batching: each session is made by `make`.
    pub fn with_continuous<C: Continuous>(
        self,
        task: &str,
        make: impl Fn() -> C + Send + Sync + 'static,
    ) -> Self {
        self.with_sessions(task, move || -> Box<dyn BatchSession> {
            Box::new(ContinuousSession(make()))
        })
    }

    fn with_sessions(
        self,
        task: &str,
        open: impl Fn() -> Box<dyn BatchSession> + Send + Sync + 'static,
    ) -> Self {
        let kind = Kind::Batch(Batch {
            task: Arc::new(FnBatchTask(open)),
            policy: BatchPolicy::default(),
        });
        self.register(task, kind)
    }

    /// Sets how the batch task `task`'s sessions run.
    pub fn batch(mut self, task: &str, policy: BatchPolicy) -> Self {
        match &mut self.registered(task).kind {
            Kind::Batch(batch) => batch.policy = policy,
            _ => panic!("task {task} is not a batch task"),
        }
        self
    }

    /// Lets at most `window` of the stream task `task`'s items be in flight at once.
    pub fn window(mut self, task: &str, window: usize) -> Self {
        match &mut self.registered(task).kind {
            Kind::Stream(stream) => stream.window = window,
            _ => panic!("task {task} is not a stream task"),
        }
        self
    }

    /// Marks `task` as a checkpoint: it has side effects that recovery must not repeat.
    pub fn checkpoint(mut self, task: &str) -> Self {
        self.registered(task).checkpoint = true;
        self
    }
}

#[async_trait]
impl Tasks for FnTasks {
    fn task(&self, task: &TaskRef) -> Option<Task> {
        self.tasks.get(&task.id).cloned()
    }
}

type Items = Box<dyn Iterator<Item = Result<Value, String>> + Send>;

struct FnCall<F>(F);

impl<F: Fn(Args) -> Result<Value, String> + Send + Sync> CallTask for FnCall<F> {
    fn call(&self, args: Args, key: CallKey) -> anyhow::Result<Result<Value, String>> {
        Ok(with_key(key, || (self.0)(args)))
    }
}

struct FnStreamTask<F>(F);

impl<F: Fn(Args) -> Result<Items, String> + Send + Sync> StreamTask for FnStreamTask<F> {
    fn open(
        self: Arc<Self>,
        args: Args,
        _cursor: Option<String>,
        key: CallKey,
    ) -> anyhow::Result<Result<Box<dyn TaskStream>, String>> {
        let items = with_key(key, || (self.0)(args));
        Ok(items.map(|items| Box::new(FnStream { items, key }) as Box<dyn TaskStream>))
    }
}

struct FnBatchTask<F>(F);

impl<F: Fn() -> Box<dyn BatchSession> + Send + Sync> BatchTask for FnBatchTask<F> {
    fn open(self: Arc<Self>) -> anyhow::Result<Result<Box<dyn BatchSession>, String>> {
        Ok(Ok((self.0)()))
    }
}

/// A dynamic batch: every step runs the task over the items given and finishes them all.
struct Dynamic<I, O, E, F> {
    f: F,
    types: PhantomData<fn(I) -> (O, E)>,
}

impl<I, O, E, F> BatchSession for Dynamic<I, O, E, F>
where
    I: Data,
    O: Data,
    E: Display,
    F: Fn(Vec<I>) -> Result<Vec<Result<O, E>>, E> + Send,
{
    fn step(&mut self, admit: Vec<(u64, Args)>) -> anyhow::Result<Finished> {
        // An item whose input does not fit fails alone; the rest run.
        let mut finished = Vec::new();
        let mut ids = Vec::new();
        let mut items = Vec::new();
        for (id, args) in admit {
            match from_args(args) {
                Ok(item) => {
                    ids.push(id);
                    items.push(item);
                }
                Err(err) => finished.push((id, Err(format!("bad input: {err:#}")))),
            }
        }
        if items.is_empty() {
            return Ok(finished);
        }
        let outputs = match (self.f)(items) {
            Ok(outputs) => outputs,
            Err(err) => {
                let message = err.to_string();
                finished.extend(ids.into_iter().map(|id| (id, Err(message.clone()))));
                return Ok(finished);
            }
        };
        anyhow::ensure!(
            outputs.len() == ids.len(),
            "a batch of {} gave {} results",
            ids.len(),
            outputs.len()
        );
        finished.extend(
            ids.into_iter()
                .zip(outputs)
                .map(|(id, output)| (id, output.map(Value::new).map_err(|err| err.to_string()))),
        );
        Ok(finished)
    }
}

struct ContinuousSession<C>(C);

impl<C: Continuous> BatchSession for ContinuousSession<C> {
    fn step(&mut self, admit: Vec<(u64, Args)>) -> anyhow::Result<Finished> {
        let mut finished = Vec::new();
        let mut items = Vec::new();
        for (id, args) in admit {
            match from_args(args) {
                Ok(item) => items.push((id, item)),
                Err(err) => finished.push((id, Err(format!("bad input: {err:#}")))),
            }
        }
        finished.extend(
            self.0
                .step(items)
                .into_iter()
                .map(|(id, output)| (id, output.map(Value::new))),
        );
        Ok(finished)
    }
}

/// An in-process stream, which has its call's key while each item is pulled.
struct FnStream {
    items: Items,
    key: CallKey,
}

impl TaskStream for FnStream {
    fn next(&mut self) -> anyhow::Result<Option<Result<StreamItem, String>>> {
        let item = with_key(self.key, || self.items.next());
        Ok(item.map(|item| {
            item.map(|value| StreamItem {
                value,
                cursor: None,
            })
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: &str) -> TaskRef {
        TaskRef {
            id: id.to_string(),
            version: String::new(),
        }
    }

    #[test]
    fn several_outputs_are_a_tuple() {
        let parts = unpack_output(Value::new((1, String::from("a"))), 2).unwrap();
        assert_eq!(parts[1].to::<String>().unwrap(), "a");
        assert_eq!(unpack_output(Value::new(vec![1, 2]), 1).unwrap().len(), 1);
        assert!(unpack_output(Value::new(1), 2).is_err());
    }

    #[test]
    fn fn_tasks_take_their_arguments_by_type() {
        let tasks = FnTasks::new()
            .with("add", |(a, b): (i32, i32)| Ok::<_, String>(a + b))
            .with("fail", |_: ()| Err::<i32, _>("no"));
        let call = |id: &str, args: Args| match tasks.task(&task(id)).map(|task| task.kind) {
            Some(Kind::Call(task)) => task.call(args, key(0)).unwrap(),
            _ => panic!("{id} is not a call task"),
        };
        assert!(tasks.task(&task("mul")).is_none());
        let sum = call("add", vec![Value::new(2), Value::new(3)]).unwrap();
        assert_eq!(sum.to::<i32>().unwrap(), 5);
        assert_eq!(call("fail", vec![]).unwrap_err(), "no");
        assert!(call("add", vec![Value::new("x".to_string())]).is_err());
    }

    fn key(call: u64) -> CallKey {
        CallKey {
            run: RunId::nil(),
            branch: BranchId { scope: 1, index: 2 },
            call,
            op: 4,
        }
    }

    #[test]
    fn an_in_process_task_reads_its_call_key() {
        let tasks = FnTasks::new().with("whoami", |_: ()| {
            Ok::<_, String>(call_key().map(|key| key.to_string()))
        });
        let Some(Kind::Call(task)) = tasks.task(&task("whoami")).map(|task| task.kind) else {
            panic!("whoami is a call task");
        };
        let named = task.call(vec![], key(5)).unwrap().unwrap();
        assert_eq!(
            named.to::<Option<String>>().unwrap().as_deref(),
            Some("00000000-0000-0000-0000-000000000000/1.2/5/4")
        );
        // Only while the call runs.
        assert_eq!(call_key(), None);
    }
}
