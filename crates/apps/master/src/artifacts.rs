//! Task artifacts: `tar.zst` packages of a manifest and the task's code, published once per
//! version and fetched by workers. They are kept in an object store: a directory on the
//! orchestrator's machine, or a bucket (S3), which outlives it and which any orchestrator reaches.

use std::io::{Cursor, Read};
use std::sync::Arc;

use anyhow::Context;
use bytes::Bytes;
use namu_proto::{TaskManifest, TaskRuntime};
use object_store::aws::AmazonS3Builder;
use object_store::local::LocalFileSystem;
use object_store::path::Path;
use object_store::prefix::PrefixStore;
use object_store::{ObjectStore, ObjectStoreExt, PutPayload};
use sha2::Digest;

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

/// Where task packages are kept: a directory, or a bucket.
pub struct Artifacts {
    store: Arc<dyn ObjectStore>,
}

impl Artifacts {
    /// In the directory `dir`, made if it is not there.
    pub fn in_dir(dir: &std::path::Path) -> anyhow::Result<Self> {
        std::fs::create_dir_all(dir).with_context(|| format!("making {}", dir.display()))?;
        let store = LocalFileSystem::new_with_prefix(dir)?;
        Ok(Self {
            store: Arc::new(store),
        })
    }

    /// At `url`: `s3://bucket/prefix`, with credentials as AWS tools find them (the environment,
    /// an ECS task's role, an instance profile), or `file:///path`.
    pub fn at(url: &str) -> anyhow::Result<Self> {
        let parsed = url::Url::parse(url).with_context(|| format!("artifacts URL {url:?}"))?;
        match parsed.scheme() {
            "s3" => {
                let bucket = AmazonS3Builder::from_env().with_url(url).build()?;
                let prefix = parsed.path().trim_matches('/');
                let store: Arc<dyn ObjectStore> = match prefix.is_empty() {
                    true => Arc::new(bucket),
                    false => Arc::new(PrefixStore::new(bucket, prefix)),
                };
                Ok(Self { store })
            }
            "file" => {
                let dir = parsed
                    .to_file_path()
                    .map_err(|()| anyhow::anyhow!("{url:?} is not a local path"))?;
                Self::in_dir(&dir)
            }
            other => {
                anyhow::bail!("artifacts at {other}:// are not supported; use s3:// or file://")
            }
        }
    }

    /// In memory, for tests.
    #[cfg(test)]
    pub fn in_memory() -> Self {
        Self {
            store: Arc::new(object_store::memory::InMemory::new()),
        }
    }

    fn path(task_id: &str, version: &str) -> anyhow::Result<Path> {
        anyhow::ensure!(
            valid_id(task_id) && valid_id(version),
            "invalid task {task_id:?} {version:?}"
        );
        Ok(Path::from(format!(
            "tasks/{task_id}/{version}/artifact.tar.zst"
        )))
    }

    /// Stores task `task_id@version`'s package. A reader sees all of it or none: a file is
    /// written aside and renamed, an object put whole.
    pub async fn put(&self, task_id: &str, version: &str, artifact: Bytes) -> anyhow::Result<()> {
        let path = Self::path(task_id, version)?;
        self.store.put(&path, PutPayload::from(artifact)).await?;
        Ok(())
    }

    pub async fn get(&self, task_id: &str, version: &str) -> anyhow::Result<Option<Bytes>> {
        let path = Self::path(task_id, version)?;
        match self.store.get(&path).await {
            Ok(found) => Ok(Some(found.bytes().await?)),
            Err(object_store::Error::NotFound { .. }) => Ok(None),
            Err(err) => Err(err).with_context(|| format!("reading {path}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn packages_are_kept_by_task_and_version() {
        let dir = std::env::temp_dir().join(format!("namu-artifacts-{}", std::process::id()));
        let on_disk = Artifacts::in_dir(&dir).unwrap();
        let by_url = Artifacts::at(&format!("file://{}", dir.display())).unwrap();
        for artifacts in [Artifacts::in_memory(), on_disk] {
            assert_eq!(artifacts.get("t", "1").await.unwrap(), None);
            artifacts
                .put("t", "1", Bytes::from_static(b"one"))
                .await
                .unwrap();
            artifacts
                .put("t", "2", Bytes::from_static(b"two"))
                .await
                .unwrap();
            assert_eq!(artifacts.get("t", "1").await.unwrap().unwrap(), "one");
            assert_eq!(artifacts.get("t", "2").await.unwrap().unwrap(), "two");
            assert!(artifacts.get("..", "1").await.is_err());
        }
        // The same directory, reached by URL.
        assert_eq!(by_url.get("t", "1").await.unwrap().unwrap(), "one");
        assert!(Artifacts::at("ftp://example.com/x").is_err());
        // A bucket, with a prefix or without: credentials are found when it is first used. Its
        // client takes the process's TLS provider, which `main` installs.
        let _ = rustls::crypto::ring::default_provider().install_default();
        assert!(Artifacts::at("s3://namu-artifacts/cluster-a").is_ok());
        assert!(Artifacts::at("s3://namu-artifacts").is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
