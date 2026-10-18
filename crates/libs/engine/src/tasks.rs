//! The tasks a worker can run.
//!
//! A task takes one JSON input and returns one JSON output; a stream task yields its outputs one
//! at a time. Arguments and results are packed by a fixed convention, shared with every task
//! runtime: no arguments is `null`, one is the value itself, several are an array; a task with
//! several outputs returns an array.

use std::collections::{HashMap, HashSet};
use std::fmt::Display;

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

/// Tasks implemented as Rust functions in this process: the fastest runtime, with no
/// serialization boundary beyond JSON values. They are not versioned.
#[derive(Default)]
pub struct FnTasks {
    tasks: HashMap<String, TaskFn>,
    streams: HashMap<String, StreamFn>,
    windows: HashMap<String, usize>,
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
        self.tasks.contains_key(&task.id) || self.streams.contains_key(&task.id)
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
