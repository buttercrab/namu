//! Task artifacts: `tar.zst` packages of a manifest and the task's code, published once per
//! version and fetched by workers. [`DiskArtifacts`] keeps them in a directory; an object store
//! (S3) goes behind the same trait.

use std::io::{Cursor, Read};
use std::path::PathBuf;

use anyhow::Context;
use async_trait::async_trait;
use bytes::Bytes;
use namu_proto::{TaskManifest, TaskRuntime};
use sha2::Digest;

#[async_trait]
pub trait Artifacts: Send + Sync + 'static {
    async fn put(&self, task_id: &str, version: &str, artifact: Bytes) -> anyhow::Result<()>;
    async fn get(&self, task_id: &str, version: &str) -> anyhow::Result<Option<Bytes>>;
}

/// Whether `id` is usable as a task or workflow id, or a version: letters, digits, `-`, `_` and
/// `.`, and not `.` or `..`. So it is safe in a path, and in a store's keys.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id != "."
        && id != ".."
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// The manifest in `artifact`, once the task's code is checked against its checksum.
pub fn manifest_of(artifact: &[u8]) -> anyhow::Result<TaskManifest> {
    let decoder = zstd::stream::read::Decoder::new(Cursor::new(artifact))
        .context("an artifact is a tar.zst package")?;
    let mut archive = tar::Archive::new(decoder);
    let mut manifest: Option<TaskManifest> = None;
    let mut digest: Option<String> = None;
    for entry in archive.entries()? {
        let mut entry = entry?;
        let name = entry
            .path()?
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf)?;
        if name == "manifest.json" {
            manifest = Some(serde_json::from_slice(&buf).context("manifest.json")?);
        } else if TaskRuntime::ALL
            .iter()
            .any(|runtime| runtime.is_artifact(&name))
        {
            digest = Some(format!("sha256:{:x}", sha2::Sha256::digest(&buf)));
        }
    }
    let manifest = manifest.context("the artifact has no manifest.json")?;
    let digest = digest.context("the artifact has no task code")?;
    anyhow::ensure!(
        manifest.checksum == digest,
        "the manifest's checksum {} is not the code's, {digest}",
        manifest.checksum
    );
    Ok(manifest)
}

pub struct DiskArtifacts {
    root: PathBuf,
}

impl DiskArtifacts {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn path(&self, task_id: &str, version: &str) -> anyhow::Result<PathBuf> {
        anyhow::ensure!(
            valid_id(task_id) && valid_id(version),
            "invalid task {task_id:?} {version:?}"
        );
        Ok(self
            .root
            .join(task_id)
            .join(version)
            .join("artifact.tar.zst"))
    }
}

#[async_trait]
impl Artifacts for DiskArtifacts {
    async fn put(&self, task_id: &str, version: &str, artifact: Bytes) -> anyhow::Result<()> {
        let path = self.path(task_id, version)?;
        if let Some(dir) = path.parent() {
            tokio::fs::create_dir_all(dir).await?;
        }
        // Written aside and renamed, so a reader never sees half an artifact.
        let partial = path.with_extension("partial");
        tokio::fs::write(&partial, &artifact).await?;
        tokio::fs::rename(&partial, &path).await?;
        Ok(())
    }

    async fn get(&self, task_id: &str, version: &str) -> anyhow::Result<Option<Bytes>> {
        let path = self.path(task_id, version)?;
        match tokio::fs::read(&path).await {
            Ok(bytes) => Ok(Some(Bytes::from(bytes))),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err).with_context(|| format!("reading {}", path.display())),
        }
    }
}

/// Artifacts in memory, for tests.
#[cfg(test)]
#[derive(Default)]
pub struct MemoryArtifacts(std::sync::Mutex<std::collections::HashMap<(String, String), Bytes>>);

#[cfg(test)]
#[async_trait]
impl Artifacts for MemoryArtifacts {
    async fn put(&self, task_id: &str, version: &str, artifact: Bytes) -> anyhow::Result<()> {
        let key = (task_id.to_string(), version.to_string());
        self.0.lock().unwrap().insert(key, artifact);
        Ok(())
    }

    async fn get(&self, task_id: &str, version: &str) -> anyhow::Result<Option<Bytes>> {
        let key = (task_id.to_string(), version.to_string());
        Ok(self.0.lock().unwrap().get(&key).cloned())
    }
}
