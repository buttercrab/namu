//! The tasks a worker can run.
//!
//! A task takes one JSON input and returns one JSON output; a stream task yields its outputs one
//! at a time; a batch task runs as a session that takes items in and gives results back in steps.
//! Arguments and results are packed by a fixed convention, shared with every task runtime: no
//! arguments is `null`, one is the value itself, several are an array; a task with several outputs
//! returns an array.

use std::collections::{HashMap, HashSet};
use std::fmt::Display;
use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::run::RunSpec;

/// A task at the version a run uses.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TaskRef {
    pub id: String,
    /// Empty for in-process tasks, which are not versioned.
    pub version: String,
}

/// Runs tasks on one worker.
#[async_trait]
pub trait Tasks: Send + Sync + 'static {
    /// Called before any branch of `run` runs on this worker, e.g. to load the tasks it uses.
    async fn prepare(&self, _run: &RunSpec) -> anyhow::Result<()> {
        Ok(())
    }

    /// Whether this worker can run `task`.
    fn serves(&self, task: &TaskRef) -> bool;

    /// Whether `task` has external side effects that must not be repeated by recovery (design
    /// §7.5). The branch is recorded past such a task before it continues.
    fn checkpoint(&self, _task: &TaskRef) -> bool {
        false
    }

    /// Whether `task` is a stream task, run with [`Tasks::open`] instead of [`Tasks::call`].
    fn is_stream(&self, _task: &TaskRef) -> bool {
        false
    }

    /// Opens a stream task, at `cursor` if one of its items gave one. Errors as for `call`.
    fn open(
        &self,
        task: &TaskRef,
        _input: Value,
        _cursor: Option<String>,
    ) -> anyhow::Result<Result<Box<dyn TaskStream>, String>> {
        anyhow::bail!("task {} is not a stream task", task.id)
    }

    /// How many of a stream's items may be in flight at once (design §6.5).
    fn window(&self, _task: &TaskRef) -> usize {
        DEFAULT_WINDOW
    }

    /// Whether `task` is a batch task, run in a session with [`Tasks::open_batch`].
    fn is_batch(&self, _task: &TaskRef) -> bool {
        false
    }

    /// Opens a session of a batch task. Errors as for `call`.
    fn open_batch(&self, task: &TaskRef) -> anyhow::Result<Result<Box<dyn BatchSession>, String>> {
        anyhow::bail!("task {} is not a batch task", task.id)
    }

    fn batch_policy(&self, _task: &TaskRef) -> BatchPolicy {
        BatchPolicy::default()
    }

    /// Runs `task` once. `Ok(Err(message))` is the task's own failure, which fails the branch;
    /// `Err` is a failure of the machinery around it.
    fn call(&self, task: &TaskRef, input: Value) -> anyhow::Result<Result<Value, String>>;
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
    fn step(&mut self, admit: Vec<(u64, Value)>) -> anyhow::Result<Finished>;
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
    type In: DeserializeOwned;
    type Out: Serialize;

    /// Takes in `admit` and returns what finished in this step.
    fn step(&mut self, admit: Vec<(u64, Self::In)>) -> Vec<(u64, Result<Self::Out, String>)>;
}

/// Packs call arguments into one task input.
pub fn pack_input(mut args: Vec<Value>) -> Value {
    match args.len() {
        0 => Value::Null,
        1 => args.pop().expect("one argument"),
        _ => Value::Array(args),
    }
}

/// Unpacks a task output into `arity` values.
pub fn unpack_output(output: Value, arity: usize) -> anyhow::Result<Vec<Value>> {
    match (arity, output) {
        (0, _) => Ok(Vec::new()),
        (1, output) => Ok(vec![output]),
        (n, Value::Array(parts)) if parts.len() == n => Ok(parts),
        (n, output) => anyhow::bail!("expected {n} outputs, got {output}"),
    }
}

type TaskFn = Box<dyn Fn(Value) -> Result<Value, String> + Send + Sync>;
type Items = Box<dyn Iterator<Item = Result<Value, String>> + Send>;
type StreamFn = Box<dyn Fn(Value) -> Result<Items, String> + Send + Sync>;
type SessionFn = Box<dyn Fn() -> Box<dyn BatchSession> + Send + Sync>;

/// Tasks implemented as Rust functions in this process: the fastest runtime, with no
/// serialization boundary beyond JSON values. They are not versioned.
#[derive(Default)]
pub struct FnTasks {
    tasks: HashMap<String, TaskFn>,
    streams: HashMap<String, StreamFn>,
    windows: HashMap<String, usize>,
    batches: HashMap<String, SessionFn>,
    policies: HashMap<String, BatchPolicy>,
    checkpoints: HashSet<String>,
}

impl FnTasks {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers `task`. `I` is the packed input: `()` for no arguments, the argument itself for
    /// one, a tuple for several.
    pub fn with<I, O, E>(
        mut self,
        task: &str,
        f: impl Fn(I) -> Result<O, E> + Send + Sync + 'static,
    ) -> Self
    where
        I: DeserializeOwned,
        O: Serialize,
        E: Display,
    {
        let call = move |input: Value| {
            let input = serde_json::from_value(input).map_err(|err| format!("bad input: {err}"))?;
            let output = f(input).map_err(|err| err.to_string())?;
            serde_json::to_value(output).map_err(|err| format!("bad output: {err}"))
        };
        self.tasks.insert(task.to_string(), Box::new(call));
        self
    }

    /// Registers a stream task: `f` returns the items, which are produced as they are pulled. The
    /// items give no cursors, so a resumed stream is opened again and skipped forward.
    pub fn with_stream<I, O, E, S>(
        mut self,
        task: &str,
        f: impl Fn(I) -> Result<S, E> + Send + Sync + 'static,
    ) -> Self
    where
        I: DeserializeOwned,
        O: Serialize,
        E: Display,
        S: Iterator<Item = Result<O, E>> + Send + 'static,
    {
        let open = move |input: Value| -> Result<Items, String> {
            let input = serde_json::from_value(input).map_err(|err| format!("bad input: {err}"))?;
            let items = f(input).map_err(|err| err.to_string())?;
            Ok(Box::new(items.map(|item| {
                let item = item.map_err(|err| err.to_string())?;
                serde_json::to_value(item).map_err(|err| format!("bad output: {err}"))
            })))
        };
        self.streams.insert(task.to_string(), Box::new(open));
        self
    }

    /// Registers a batch task run by dynamic batching: each step calls `f` on the items given.
    pub fn with_batch<I, O, E>(
        mut self,
        task: &str,
        f: impl Fn(Vec<I>) -> Result<Vec<Result<O, E>>, E> + Send + Sync + 'static,
    ) -> Self
    where
        I: DeserializeOwned + 'static,
        O: Serialize + 'static,
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
        self.batches.insert(task.to_string(), Box::new(open));
        self
    }

    /// Registers a batch task run by continuous batching: each session is made by `make`.
    pub fn with_continuous<C: Continuous>(
        mut self,
        task: &str,
        make: impl Fn() -> C + Send + Sync + 'static,
    ) -> Self {
        let open = move || -> Box<dyn BatchSession> { Box::new(ContinuousSession(make())) };
        self.batches.insert(task.to_string(), Box::new(open));
        self
    }

    /// Sets how `task`'s batch sessions run.
    pub fn batch(mut self, task: &str, policy: BatchPolicy) -> Self {
        self.policies.insert(task.to_string(), policy);
        self
    }

    /// Lets at most `window` of `task`'s items be in flight at once.
    pub fn window(mut self, task: &str, window: usize) -> Self {
        self.windows.insert(task.to_string(), window);
        self
    }

    /// Marks `task` as a checkpoint: it has side effects that recovery must not repeat.
    pub fn checkpoint(mut self, task: &str) -> Self {
        self.checkpoints.insert(task.to_string());
        self
    }
}

#[async_trait]
impl Tasks for FnTasks {
    fn serves(&self, task: &TaskRef) -> bool {
        self.tasks.contains_key(&task.id)
            || self.streams.contains_key(&task.id)
            || self.batches.contains_key(&task.id)
    }

    fn is_batch(&self, task: &TaskRef) -> bool {
        self.batches.contains_key(&task.id)
    }

    fn open_batch(&self, task: &TaskRef) -> anyhow::Result<Result<Box<dyn BatchSession>, String>> {
        let open = self
            .batches
            .get(&task.id)
            .ok_or_else(|| anyhow::anyhow!("batch task {} is not served here", task.id))?;
        Ok(Ok(open()))
    }

    fn batch_policy(&self, task: &TaskRef) -> BatchPolicy {
        self.policies.get(&task.id).copied().unwrap_or_default()
    }

    fn is_stream(&self, task: &TaskRef) -> bool {
        self.streams.contains_key(&task.id)
    }

    fn open(
        &self,
        task: &TaskRef,
        input: Value,
        _cursor: Option<String>,
    ) -> anyhow::Result<Result<Box<dyn TaskStream>, String>> {
        let open = self
            .streams
            .get(&task.id)
            .ok_or_else(|| anyhow::anyhow!("stream task {} is not served here", task.id))?;
        Ok(open(input).map(|items| Box::new(FnStream(items)) as Box<dyn TaskStream>))
    }

    fn window(&self, task: &TaskRef) -> usize {
        self.windows
            .get(&task.id)
            .copied()
            .unwrap_or(DEFAULT_WINDOW)
    }

    fn checkpoint(&self, task: &TaskRef) -> bool {
        self.checkpoints.contains(&task.id)
    }

    fn call(&self, task: &TaskRef, input: Value) -> anyhow::Result<Result<Value, String>> {
        let f = self
            .tasks
            .get(&task.id)
            .ok_or_else(|| anyhow::anyhow!("task {} is not served here", task.id))?;
        Ok(f(input))
    }
}

/// A dynamic batch: every step runs the task over the items given and finishes them all.
struct Dynamic<I, O, E, F> {
    f: F,
    types: PhantomData<fn(I) -> (O, E)>,
}

impl<I, O, E, F> BatchSession for Dynamic<I, O, E, F>
where
    I: DeserializeOwned,
    O: Serialize,
    E: Display,
    F: Fn(Vec<I>) -> Result<Vec<Result<O, E>>, E> + Send,
{
    fn step(&mut self, admit: Vec<(u64, Value)>) -> anyhow::Result<Finished> {
        // An item whose input does not decode fails alone; the rest run.
        let mut finished = Vec::new();
        let mut ids = Vec::new();
        let mut items = Vec::new();
        for (id, input) in admit {
            match serde_json::from_value(input) {
                Ok(item) => {
                    ids.push(id);
                    items.push(item);
                }
                Err(err) => finished.push((id, Err(format!("bad input: {err}")))),
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
        finished.extend(ids.into_iter().zip(outputs).map(|(id, output)| {
            let output = output.map_err(|err| err.to_string()).and_then(|output| {
                serde_json::to_value(output).map_err(|err| format!("bad output: {err}"))
            });
            (id, output)
        }));
        Ok(finished)
    }
}

struct ContinuousSession<C>(C);

impl<C: Continuous> BatchSession for ContinuousSession<C> {
    fn step(&mut self, admit: Vec<(u64, Value)>) -> anyhow::Result<Finished> {
        let mut finished = Vec::new();
        let mut items = Vec::new();
        for (id, input) in admit {
            match serde_json::from_value(input) {
                Ok(item) => items.push((id, item)),
                Err(err) => finished.push((id, Err(format!("bad input: {err}")))),
            }
        }
        finished.extend(self.0.step(items).into_iter().map(|(id, output)| {
            let output = output.and_then(|output| {
                serde_json::to_value(output).map_err(|err| format!("bad output: {err}"))
            });
            (id, output)
        }));
        Ok(finished)
    }
}

struct FnStream(Items);

impl TaskStream for FnStream {
    fn next(&mut self) -> anyhow::Result<Option<Result<StreamItem, String>>> {
        Ok(self.0.next().map(|item| {
            item.map(|value| StreamItem {
                value,
                cursor: None,
            })
        }))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn task(id: &str) -> TaskRef {
        TaskRef {
            id: id.to_string(),
            version: String::new(),
        }
    }

    #[test]
    fn arguments_follow_the_packing_convention() {
        assert_eq!(pack_input(vec![]), Value::Null);
        assert_eq!(pack_input(vec![json!(3)]), json!(3));
        assert_eq!(pack_input(vec![json!(1), json!("a")]), json!([1, "a"]));
        assert_eq!(
            unpack_output(json!([1, 2]), 1).unwrap(),
            vec![json!([1, 2])]
        );
        assert_eq!(
            unpack_output(json!([1, 2]), 2).unwrap(),
            vec![json!(1), json!(2)]
        );
        assert!(unpack_output(json!(1), 2).is_err());
    }

    #[test]
    fn fn_tasks_decode_packed_inputs() {
        let tasks = FnTasks::new()
            .with("add", |(a, b): (i32, i32)| Ok::<_, String>(a + b))
            .with("fail", |_: ()| Err::<i32, _>("no"));
        assert!(tasks.serves(&task("add")));
        assert!(!tasks.serves(&task("mul")));
        assert_eq!(
            tasks.call(&task("add"), json!([2, 3])).unwrap(),
            Ok(json!(5))
        );
        assert_eq!(
            tasks.call(&task("fail"), Value::Null).unwrap(),
            Err("no".into())
        );
        assert!(tasks.call(&task("add"), json!("x")).unwrap().is_err());
    }
}
