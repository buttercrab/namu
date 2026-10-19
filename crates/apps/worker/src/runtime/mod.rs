//! Task runtimes and the artifact cache. Each task picks its runtime in its manifest.

pub mod native;
pub mod wasm;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use namu_engine::tasks::{Batch, BatchPolicy, DEFAULT_WINDOW, Kind, Stream};
use namu_proto::{TaskKind, TaskManifest, TaskRuntime};
use serde_json::Value;
use sha2::Digest;

/// A call's output as the runtimes give it, JSON or the task's own error, decoded.
fn output(output: Result<String, String>) -> anyhow::Result<Result<Value, String>> {
    match output {
        Ok(output) => Ok(Ok(
            serde_json::from_str(&output).context("the task returned invalid JSON")?
        )),
        Err(message) => Ok(Err(message)),
    }
}

/// Downloads, verifies and loads task artifacts. Artifacts are cached on disk, so a task version
/// is downloaded once per machine.
pub struct Runtimes {
    http: reqwest::Client,
    orchestrator_url: String,
    cache_dir: PathBuf,
    engine: wasmtime::Engine,
}

impl Runtimes {
    pub fn new(http: reqwest::Client, orchestrator_url: String, cache_dir: PathBuf) -> Self {
        Self {
            http,
            orchestrator_url,
            cache_dir,
            engine: wasmtime::Engine::default(),
        }
    }

    /// Loads the task a manifest describes, as its runtime and kind say.
    pub async fn load(&self, manifest: &TaskManifest) -> anyhow::Result<Kind> {
        let path = self.fetch_artifact(manifest).await?;
        let engine = self.engine.clone();
        let runtime = manifest.runtime;
        let kind = manifest.task_kind.clone();
        let window = manifest.window.unwrap_or(DEFAULT_WINDOW);
        let policy = batch_policy(manifest);
        tokio::task::spawn_blocking(move || -> anyhow::Result<Kind> {
            Ok(match (kind, runtime) {
                (TaskKind::Single, TaskRuntime::Wasm) => {
                    Kind::Call(Arc::new(wasm::WasmTask::load(&engine, &path)?))
                }
                (TaskKind::Single, TaskRuntime::Native) => {
                    Kind::Call(Arc::new(native::NativeTask::load(&path)?))
                }
                (TaskKind::Stream, runtime) => Kind::Stream(Stream {
                    task: match runtime {
                        TaskRuntime::Wasm => Arc::new(wasm::WasmStreamTask::load(&engine, &path)?),
                        TaskRuntime::Native => Arc::new(native::NativeStreamTask::load(&path)?),
                    },
                    window,
                }),
                (TaskKind::Batch, runtime) => Kind::Batch(Batch {
                    task: match runtime {
                        TaskRuntime::Wasm => Arc::new(wasm::WasmBatchTask::load(&engine, &path)?),
                        TaskRuntime::Native => Arc::new(native::NativeBatchTask::load(&path)?),
                    },
                    policy,
                }),
            })
        })
        .await?
    }

    async fn fetch_artifact(&self, manifest: &TaskManifest) -> anyhow::Result<PathBuf> {
        let dir = self
            .cache_dir
            .join(safe_segment(&manifest.task_id)?)
            .join(safe_segment(&manifest.version)?);
        tokio::fs::create_dir_all(&dir).await?;
        let archive = dir.join("artifact.tar.zst");
        if !archive.exists() {
            let bytes = self
                .http
                .get(format!(
                    "{}/tasks/{}/{}/artifact",
                    self.orchestrator_url, manifest.task_id, manifest.version
                ))
                .send()
                .await?
                .error_for_status()?
                .bytes()
                .await?;
            tokio::fs::write(&archive, &bytes).await?;
        }
        let runtime = manifest.runtime;
        let checksum = manifest.checksum.clone();
        let dir_clone = dir.clone();
        tokio::task::spawn_blocking(move || extract(&archive, &dir_clone, runtime, &checksum))
            .await?
    }
}

fn batch_policy(manifest: &TaskManifest) -> BatchPolicy {
    let defaults = BatchPolicy::default();
    BatchPolicy {
        size: manifest.batch_size.unwrap_or(defaults.size),
        max_wait: manifest.max_wait_ms.map(Duration::from_millis),
        slots: manifest.batch_slots.unwrap_or(defaults.slots),
    }
}

/// Extracts the task binary and verifies it against the manifest checksum.
fn extract(
    archive: &Path,
    dir: &Path,
    runtime: TaskRuntime,
    checksum: &str,
) -> anyhow::Result<PathBuf> {
    let data = std::fs::read(archive)?;
    let decoder = zstd::stream::read::Decoder::new(std::io::Cursor::new(data))?;
    let mut tar = tar::Archive::new(decoder);
    for entry in tar.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_path_buf();
        let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else {
            continue;
        };
        if !is_binary(&name, runtime) {
            continue;
        }
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut bytes)?;
        let digest = format!("sha256:{:x}", sha2::Sha256::digest(&bytes));
        if !checksum.is_empty() && digest != checksum {
            anyhow::bail!("checksum mismatch for {name}: expected {checksum}, got {digest}");
        }
        let out = dir.join(&name);
        std::fs::write(&out, &bytes).with_context(|| format!("write {}", out.display()))?;
        return Ok(out);
    }
    anyhow::bail!("artifact has no {} binary", runtime.as_str())
}

fn is_binary(name: &str, runtime: TaskRuntime) -> bool {
    match runtime {
        TaskRuntime::Wasm => name.ends_with(".wasm"),
        TaskRuntime::Native => {
            name.ends_with(".so") || name.ends_with(".dylib") || name.ends_with(".dll")
        }
    }
}

/// Rejects ids that could escape the cache directory.
pub fn safe_segment(raw: &str) -> anyhow::Result<&str> {
    let ok = !raw.is_empty()
        && raw != "."
        && raw != ".."
        && raw
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if ok {
        Ok(raw)
    } else {
        anyhow::bail!("invalid id {raw:?}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_cannot_escape_the_cache() {
        assert!(safe_segment("analyze").is_ok());
        assert!(safe_segment("0.1.0").is_ok());
        for bad in ["", ".", "..", "../etc", "a/b", "a\\b"] {
            assert!(safe_segment(bad).is_err(), "{bad:?} should be rejected");
        }
    }
}
