use std::path::{Path, PathBuf};

use bytes::Bytes;
use tokio::fs;

/// Rejects ids that could escape the artifacts directory.
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

fn artifact_path(root: &Path, task_id: &str, version: &str) -> anyhow::Result<PathBuf> {
    Ok(root
        .join(safe_segment(task_id)?)
        .join(safe_segment(version)?)
        .join("artifact.tar.zst"))
}

pub async fn store_artifact(
    root: &Path,
    task_id: &str,
    version: &str,
    bytes: &Bytes,
) -> anyhow::Result<()> {
    let path = artifact_path(root, task_id, version)?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).await?;
    }
    fs::write(&path, bytes).await?;
    Ok(())
}

pub async fn load_artifact(root: &Path, task_id: &str, version: &str) -> anyhow::Result<Bytes> {
    let path = artifact_path(root, task_id, version)?;
    Ok(Bytes::from(fs::read(&path).await?))
}
