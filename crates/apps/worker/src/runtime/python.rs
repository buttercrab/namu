//! Runs Python tasks (written with the `namu` SDK, `sdk/python`) in Python processes, each of
//! which imports the task's module and speaks the SDK's protocol on its stdin and stdout
//! (`namu.worker`):
//!
//! - calls of a task go to a pool of processes, one call at a time each; the worker awaits a call
//!   while its process works, as for any async task;
//! - an open stream, or a batch session, has a process of its own for as long as it lasts, so what
//!   the task loads once (a model) serves every item.
//!
//! A process that dies fails what it was doing, and is not reused.
//!
//! The SDK's protocol code comes from the worker itself, written to its cache, so the two always
//! agree; the tasks run in the worker's Python environment (`NAMU_PYTHON`, default `python3`).

use std::fmt;
use std::io::{BufReader, BufWriter, Read, Write};
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};

use anyhow::Context;
use async_trait::async_trait;
use namu_core::value::{Value, encode_args};
use namu_engine::tasks::{
    Admitted, Args, Async, AsyncTask, BatchSession, BatchTask, CallKey, Finished, StreamItem,
    StreamTask, TaskStream,
};
use serde::de::{self, DeserializeOwned, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use serde_bytes::ByteBuf;

/// The SDK, as the worker writes it for its Python processes.
const SDK: [(&str, &str); 3] = [
    (
        "__init__.py",
        include_str!("../../../../../sdk/python/namu/__init__.py"),
    ),
    (
        "_msgpack.py",
        include_str!("../../../../../sdk/python/namu/_msgpack.py"),
    ),
    (
        "worker.py",
        include_str!("../../../../../sdk/python/namu/worker.py"),
    ),
];

/// Python processes per task on a worker for its calls, unless `NAMU_PYTHON_PROCESSES` says
/// otherwise.
const DEFAULT_PROCESSES: usize = 4;

pub struct PythonTask {
    task_id: String,
    module: PathBuf,
    /// Where the SDK is, for `PYTHONPATH`.
    sdk: PathBuf,
    python: String,
    /// Processes between calls.
    idle: Mutex<Vec<Process>>,
}

impl PythonTask {
    /// The task `task_id` of the module at `module`.
    pub fn load(task_id: &str, module: &Path, cache_dir: &Path) -> anyhow::Result<Self> {
        let sdk = cache_dir.join("python");
        let package = sdk.join("namu");
        std::fs::create_dir_all(&package)?;
        for (name, source) in SDK {
            std::fs::write(package.join(name), source)?;
        }
        Ok(Self {
            task_id: task_id.to_string(),
            module: module.to_path_buf(),
            sdk,
            python: std::env::var("NAMU_PYTHON").unwrap_or_else(|_| "python3".to_string()),
            idle: Mutex::new(Vec::new()),
        })
    }

    /// The task as a single one: as many calls in flight as it has processes.
    pub fn calls(self) -> Async {
        let processes = std::env::var("NAMU_PYTHON_PROCESSES")
            .ok()
            .and_then(|n| n.parse().ok())
            .unwrap_or(DEFAULT_PROCESSES);
        Async {
            task: Arc::new(self),
            limit: Arc::new(tokio::sync::Semaphore::new(processes.max(1))),
        }
    }

    fn start(&self) -> anyhow::Result<Process> {
        let mut child = Command::new(&self.python)
            .args(["-m", "namu.worker"])
            .arg(&self.module)
            .env("PYTHONPATH", &self.sdk)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // What the task prints, and its tracebacks, go to the worker's log.
            .stderr(Stdio::inherit())
            .spawn()
            .with_context(|| format!("starting {} for task {}", self.python, self.task_id))?;
        let stdin = BufWriter::new(child.stdin.take().context("the process's stdin")?);
        let stdout = BufReader::new(child.stdout.take().context("the process's stdout")?);
        Ok(Process {
            child,
            stdin,
            stdout,
        })
    }

    /// A process of its own, opened with `request` (to serve a stream or a batch session).
    fn dedicated(&self, request: &impl Serialize) -> anyhow::Result<Result<Process, String>> {
        let mut process = self.start()?;
        Ok(match process.ask::<()>(request) {
            Ok(Ok(())) => Ok(process),
            Ok(Err(message)) => Err(message),
            Err(err) => Err(died(err)),
        })
    }
}

struct Process {
    child: Child,
    stdin: BufWriter<ChildStdin>,
    stdout: BufReader<ChildStdout>,
}

impl Process {
    /// Sends `request` and reads the answer, each a MessagePack document after its length. `Err`
    /// is the process gone (it crashed, or was killed) or not keeping to the protocol.
    fn ask<T: DeserializeOwned>(
        &mut self,
        request: &impl Serialize,
    ) -> anyhow::Result<Result<T, String>> {
        let request = rmp_serde::to_vec(request)?;
        self.stdin
            .write_all(&u32::try_from(request.len())?.to_be_bytes())?;
        self.stdin.write_all(&request)?;
        self.stdin.flush()?;
        let mut length = [0; 4];
        self.stdout.read_exact(&mut length)?;
        let mut answer = vec![0; u32::from_be_bytes(length) as usize];
        self.stdout.read_exact(&mut answer)?;
        let Reply(answer) = rmp_serde::from_slice(&answer)?;
        Ok(answer)
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn died(err: anyhow::Error) -> String {
    format!("the Python process died: {err:#}")
}

/// An answer: `[true, value]`, or `[false, error]` for the task's own failure.
struct Reply<T>(Result<T, String>);

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Reply<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Pair<T>(PhantomData<T>);

        impl<'de, T: Deserialize<'de>> Visitor<'de> for Pair<T> {
            type Value = Reply<T>;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("[true, value] or [false, error]")
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Reply<T>, A::Error> {
                let missing = || de::Error::invalid_length(1, &"two elements");
                let ok: bool = seq.next_element()?.ok_or_else(missing)?;
                Ok(Reply(match ok {
                    true => Ok(seq.next_element()?.ok_or_else(missing)?),
                    false => Err(seq.next_element()?.ok_or_else(missing)?),
                }))
            }
        }

        deserializer.deserialize_seq(Pair(PhantomData))
    }
}

/// A call's input as a request carries it: MessagePack, as the task ABI packs it, as binary.
fn input(args: &Args) -> anyhow::Result<ByteBuf> {
    Ok(ByteBuf::from(encode_args(args)?.to_vec()))
}

#[async_trait]
impl AsyncTask for PythonTask {
    async fn call(&self, args: Args, key: CallKey) -> anyhow::Result<Result<Value, String>> {
        let request = ("call", self.task_id.clone(), key.to_string(), input(&args)?);
        let pooled = self.idle.lock().unwrap().pop();
        let mut process = match pooled {
            Some(process) => process,
            None => self.start()?,
        };
        let (process, answer) = tokio::task::spawn_blocking(move || {
            let answer = process.ask::<ByteBuf>(&request);
            (process, answer)
        })
        .await?;
        let answer = match answer {
            Ok(answer) => answer,
            // The call fails, as a task that traps does; a new process takes this one's place.
            Err(err) => return Ok(Err(died(err))),
        };
        self.idle.lock().unwrap().push(process);
        Ok(super::output(answer.map(ByteBuf::into_vec)))
    }
}

impl StreamTask for PythonTask {
    fn open(
        self: Arc<Self>,
        args: Args,
        cursor: Option<String>,
        key: CallKey,
    ) -> anyhow::Result<Result<Box<dyn TaskStream>, String>> {
        let request = (
            "open",
            &self.task_id,
            key.to_string(),
            input(&args)?,
            cursor,
        );
        Ok(self
            .dedicated(&request)?
            .map(|process| Box::new(PythonItems(process)) as Box<dyn TaskStream>))
    }
}

/// An open stream: its process.
struct PythonItems(Process);

impl TaskStream for PythonItems {
    fn next(&mut self) -> anyhow::Result<Option<Result<StreamItem, String>>> {
        let item = match self.0.ask::<Option<(ByteBuf, Option<String>)>>(&("next",)) {
            Ok(item) => item,
            Err(err) => return Ok(Some(Err(died(err)))),
        };
        Ok(item.transpose().map(|item| {
            item.map(|(value, cursor)| StreamItem {
                value: Value::encoded(value.into_vec()),
                cursor,
            })
        }))
    }
}

impl BatchTask for PythonTask {
    fn open(self: Arc<Self>) -> anyhow::Result<Result<Box<dyn BatchSession>, String>> {
        Ok(self
            .dedicated(&("session", &self.task_id))?
            .map(|process| Box::new(PythonSession(process)) as Box<dyn BatchSession>))
    }
}

/// An open batch session: its process.
struct PythonSession(Process);

impl BatchSession for PythonSession {
    fn step(&mut self, admit: Vec<Admitted>) -> anyhow::Result<Finished> {
        let admit = admit
            .iter()
            .map(|Admitted { id, args, key }| Ok((id, key.to_string(), input(args)?)))
            .collect::<anyhow::Result<Vec<_>>>()?;
        // A failure of the step itself (or of its process) fails the session.
        let finished = self
            .0
            .ask::<Vec<(u64, Reply<ByteBuf>)>>(&("step", admit))?
            .map_err(anyhow::Error::msg)?;
        Ok(finished
            .into_iter()
            .map(|(id, Reply(output))| (id, super::output(output.map(ByteBuf::into_vec))))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use namu_engine::ids::RunId;

    use super::*;

    const MODULE: &str = r#"
import os
from dataclasses import dataclass
import namu

@dataclass
class Stats:
    words: int
    top: str

@namu.task
def merge(a: Stats, b: Stats) -> Stats:
    return Stats(a.words + b.words, max(a.top, b.top))

@namu.task
def whoami() -> str:
    return namu.call_key()

@namu.task
def fails(x: int) -> int:
    raise ValueError(f"no {x}")

@namu.task
def crashes() -> int:
    os._exit(3)

@namu.stream
def count(n: int, *, cursor: str | None = None):
    for i in range(int(cursor or 0), n):
        yield namu.Item(f"{i}:{namu.call_key()}", cursor=str(i + 1))

@namu.stream
def broken(n: int):
    yield n
    raise RuntimeError("the source went away")

@namu.batch
def double(xs: list[Stats]) -> list[Stats]:
    return [ValueError("odd") if s.words % 2 else Stats(s.words * 2, s.top) for s in xs]

@namu.continuous
class Countdown:
    """Each word stays for as many steps as it has letters."""

    def __init__(self):
        self.active = {}

    def step(self, admit: list[tuple[int, str]]) -> list[tuple[int, str]]:
        for id, word in admit:
            self.active[id] = [word, len(word)]
        done = []
        for id, entry in list(self.active.items()):
            entry[1] -= 1
            if entry[1] == 0:
                done.append((id, entry[0].upper()))
                del self.active[id]
        return done
"#;

    fn key(op: usize) -> CallKey {
        CallKey {
            run: RunId::nil(),
            lineage: 1,
            clock: 2,
            op,
        }
    }

    fn load(dir: &Path, task_id: &str) -> Arc<PythonTask> {
        let module = dir.join("task.py");
        std::fs::write(&module, MODULE).unwrap();
        Arc::new(PythonTask::load(task_id, &module, dir).unwrap())
    }

    fn task(dir: &Path, task_id: &str) -> Async {
        let module = dir.join("task.py");
        std::fs::write(&module, MODULE).unwrap();
        PythonTask::load(task_id, &module, dir).unwrap().calls()
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("namu-python-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn drain(stream: &mut dyn TaskStream) -> Vec<(String, Option<String>)> {
        let mut items = Vec::new();
        while let Some(item) = stream.next().unwrap() {
            let item = item.unwrap();
            items.push((item.value.take::<String>().unwrap(), item.cursor));
        }
        items
    }

    #[test]
    fn python_streams_yield_items_and_reopen_at_a_cursor() {
        let dir = scratch("stream");
        let count = load(&dir, "count");
        let mut stream = StreamTask::open(count.clone(), vec![Value::new(3)], None, key(1))
            .unwrap()
            .unwrap();
        let k = key(1).to_string();
        assert_eq!(
            drain(stream.as_mut()),
            [0, 1, 2].map(|i| (format!("{i}:{k}"), Some((i + 1).to_string())))
        );

        // Reopened after its second item, it goes on from the third.
        let mut stream = StreamTask::open(count, vec![Value::new(3)], Some("2".into()), key(1))
            .unwrap()
            .unwrap();
        assert_eq!(
            drain(stream.as_mut()),
            [(format!("2:{k}"), Some("3".into()))]
        );

        // A stream's own error ends it, after what it yielded.
        let broken = load(&dir, "broken");
        let mut stream = StreamTask::open(broken, vec![Value::new(7)], None, key(2))
            .unwrap()
            .unwrap();
        assert_eq!(
            stream
                .next()
                .unwrap()
                .unwrap()
                .unwrap()
                .value
                .take::<i64>()
                .unwrap(),
            7
        );
        let err = stream.next().unwrap().unwrap().err().unwrap();
        assert_eq!(err, "RuntimeError: the source went away");

        // Not a stream task: it does not open.
        let merge = load(&dir, "merge");
        let err = StreamTask::open(merge, vec![], None, key(3))
            .unwrap()
            .err()
            .unwrap();
        assert_eq!(err, "TypeError: merge is a single task, not a stream one");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn admit(items: Vec<(u64, Value)>) -> Vec<Admitted> {
        items
            .into_iter()
            .map(|(id, value)| Admitted {
                id,
                args: vec![value],
                key: key(id as usize),
            })
            .collect()
    }

    fn outputs<T: namu_core::value::Data>(finished: Finished) -> Vec<(u64, Result<T, String>)> {
        let mut outputs: Vec<_> = finished
            .into_iter()
            .map(|(id, output)| (id, output.map(|value| value.take::<T>().unwrap())))
            .collect();
        outputs.sort_by_key(|(id, _)| *id);
        outputs
    }

    #[test]
    fn python_batch_sessions_keep_their_process_across_steps() {
        let dir = scratch("batch");
        let stats = |words| Stats {
            words,
            top: "a".into(),
        };

        // Dynamic: every item a step is given finishes in it; an exception fails its item alone.
        let mut session = BatchTask::open(load(&dir, "double")).unwrap().unwrap();
        let step = session
            .step(admit(vec![
                (1, Value::new(stats(2))),
                (2, Value::new(stats(3))),
            ]))
            .unwrap();
        assert_eq!(
            outputs::<Stats>(step),
            [(1, Ok(stats(4))), (2, Err("ValueError: odd".into()))]
        );

        // Continuous: a word leaves the session after as many steps as it has letters, so a
        // short one admitted later can finish first.
        let mut session = BatchTask::open(load(&dir, "Countdown")).unwrap().unwrap();
        let first = session
            .step(admit(vec![(1, Value::new("abc".to_string()))]))
            .unwrap();
        assert!(first.is_empty());
        let second = session
            .step(admit(vec![(2, Value::new("x".to_string()))]))
            .unwrap();
        assert_eq!(outputs::<String>(second), [(2, Ok("X".into()))]);
        let third = session.step(Vec::new()).unwrap();
        assert_eq!(outputs::<String>(third), [(1, Ok("ABC".into()))]);

        // Not a batch task: no session.
        let err = BatchTask::open(load(&dir, "whoami"))
            .unwrap()
            .err()
            .unwrap();
        assert_eq!(
            err,
            "TypeError: whoami is a single task, not a batch or continuous one"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
    struct Stats {
        words: u32,
        top: String,
    }

    impl namu_core::value::Data for Stats {}

    #[tokio::test]
    async fn python_tasks_run_in_a_pool_of_processes() {
        let dir = scratch("calls");

        let merge = task(&dir, "merge");
        let (a, b) = (
            Stats {
                words: 3,
                top: "a".into(),
            },
            Stats {
                words: 4,
                top: "b".into(),
            },
        );
        let merged = merge
            .call(vec![Value::new(a), Value::new(b)], key(0))
            .await
            .unwrap();
        assert_eq!(
            merged.unwrap().take::<Stats>().unwrap(),
            Stats {
                words: 7,
                top: "b".into()
            }
        );

        let whoami = task(&dir, "whoami");
        let named = whoami.call(vec![], key(5)).await.unwrap().unwrap();
        assert_eq!(named.take::<String>().unwrap(), key(5).to_string());

        let fails = task(&dir, "fails");
        let failed = fails.call(vec![Value::new(4)], key(0)).await.unwrap();
        assert_eq!(failed.unwrap_err(), "ValueError: no 4");

        // A process that dies fails its call; the next call gets a new one.
        let crashes = task(&dir, "crashes");
        let crashed = crashes.call(vec![], key(0)).await.unwrap();
        assert!(crashed.unwrap_err().contains("the Python process died"));
        let crashed = crashes.call(vec![], key(0)).await.unwrap();
        assert!(crashed.unwrap_err().contains("the Python process died"));

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
