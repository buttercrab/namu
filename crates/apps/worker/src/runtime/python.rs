//! Runs Python tasks (written with the `namu` SDK, `sdk/python`) in a pool of Python processes:
//! each process imports the task's module and runs one call at a time, speaking the SDK's
//! protocol on its stdin and stdout (`namu.worker`). The worker awaits a call while its process
//! works, as for any async task. A process that dies fails its call, and is not reused.
//!
//! The SDK's protocol code comes from the worker itself, written to its cache, so the two always
//! agree; the tasks run in the worker's Python environment (`NAMU_PYTHON`, default `python3`).

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};

use anyhow::Context;
use async_trait::async_trait;
use namu_core::value::{Value, encode_args};
use namu_engine::tasks::{Args, Async, AsyncTask, CallKey};
use serde::Deserialize;
use serde_bytes::ByteBuf;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

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

/// Python processes per task on a worker, unless `NAMU_PYTHON_PROCESSES` says otherwise.
const DEFAULT_PROCESSES: usize = 4;

pub struct PythonTask {
    task_id: String,
    module: PathBuf,
    /// Where the SDK is, for `PYTHONPATH`.
    sdk: PathBuf,
    python: String,
    idle: Mutex<Vec<Process>>,
}

struct Process {
    // Killed when dropped.
    _child: Child,
    stdin: ChildStdin,
    stdout: ChildStdout,
}

impl PythonTask {
    /// The task `task_id` of the module at `module`, as an async task with as many calls in
    /// flight as it has processes.
    pub fn load(task_id: &str, module: &Path, cache_dir: &Path) -> anyhow::Result<Async> {
        let sdk = cache_dir.join("python");
        let package = sdk.join("namu");
        std::fs::create_dir_all(&package)?;
        for (name, source) in SDK {
            std::fs::write(package.join(name), source)?;
        }
        let processes = std::env::var("NAMU_PYTHON_PROCESSES")
            .ok()
            .and_then(|n| n.parse().ok())
            .unwrap_or(DEFAULT_PROCESSES);
        let task = Self {
            task_id: task_id.to_string(),
            module: module.to_path_buf(),
            sdk,
            python: std::env::var("NAMU_PYTHON").unwrap_or_else(|_| "python3".to_string()),
            idle: Mutex::new(Vec::new()),
        };
        Ok(Async {
            task: Arc::new(task),
            limit: Arc::new(tokio::sync::Semaphore::new(processes.max(1))),
        })
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
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("starting {} for task {}", self.python, self.task_id))?;
        let stdin = child.stdin.take().context("the process's stdin")?;
        let stdout = child.stdout.take().context("the process's stdout")?;
        Ok(Process {
            _child: child,
            stdin,
            stdout,
        })
    }
}

/// A call's result as a process sends it: `[true, output]` (MessagePack, as binary) or `[false,
/// error]` (text). Both read as bytes; the flag says which they are.
#[derive(Deserialize)]
struct Answer(bool, ByteBuf);

#[async_trait]
impl AsyncTask for PythonTask {
    async fn call(&self, args: Args, key: CallKey) -> anyhow::Result<Result<Value, String>> {
        let input = encode_args(&args)?;
        let request = rmp_serde::to_vec(&(
            &self.task_id,
            key.to_string(),
            ByteBuf::from(input.to_vec()),
        ))?;
        let pooled = self.idle.lock().unwrap().pop();
        let mut process = match pooled {
            Some(process) => process,
            None => self.start()?,
        };
        let answer = match exchange(&mut process, &request).await {
            Ok(answer) => answer,
            // The process is gone (it crashed, or was killed): the call fails, as a task that
            // traps does, and a new process takes its place next time.
            Err(err) => return Ok(Err(format!("the Python process died: {err:#}"))),
        };
        self.idle.lock().unwrap().push(process);
        let Answer(ok, payload) = answer;
        Ok(match ok {
            true => super::output(Ok(payload.into_vec())),
            false => Err(String::from_utf8_lossy(&payload).into_owned()),
        })
    }
}

/// Sends one call to `process` and reads its answer, each a MessagePack document after its
/// length.
async fn exchange(process: &mut Process, request: &[u8]) -> anyhow::Result<Answer> {
    process
        .stdin
        .write_all(&(request.len() as u32).to_be_bytes())
        .await?;
    process.stdin.write_all(request).await?;
    process.stdin.flush().await?;
    let mut length = [0; 4];
    process.stdout.read_exact(&mut length).await?;
    let mut answer = vec![0; u32::from_be_bytes(length) as usize];
    process.stdout.read_exact(&mut answer).await?;
    Ok(rmp_serde::from_slice(&answer)?)
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
"#;

    fn key(op: usize) -> CallKey {
        CallKey {
            run: RunId::nil(),
            lineage: 1,
            clock: 2,
            op,
        }
    }

    fn task(dir: &Path, task_id: &str) -> Async {
        let module = dir.join("task.py");
        std::fs::write(&module, MODULE).unwrap();
        PythonTask::load(task_id, &module, dir).unwrap()
    }

    #[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
    struct Stats {
        words: u32,
        top: String,
    }

    impl namu_core::value::Data for Stats {}

    #[tokio::test]
    async fn python_tasks_run_in_a_pool_of_processes() {
        let dir = std::env::temp_dir().join(format!("namu-python-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

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
