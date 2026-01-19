use std::path::{Path, PathBuf};

use bytes::Bytes;
use tokio::fs;

pub async fn store_artifact(
    root: &Path,
    task_id: &str,
    version: &str,
    bytes: &Bytes,
) -> anyhow::Result<PathBuf> {
    let dir = root.join(task_id).join(version);
    fs::create_dir_all(&dir).await?;
    let path = dir.join("artifact.tar.zst");
    fs::write(&path, bytes).await?;
    Ok(path)
}

pub async fn load_artifact(root: &Path, task_id: &str, version: &str) -> anyhow::Result<Bytes> {
    let path = root.join(task_id).join(version).join("artifact.tar.zst");
    let bytes = fs::read(&path).await?;
    Ok(Bytes::from(bytes))
}
