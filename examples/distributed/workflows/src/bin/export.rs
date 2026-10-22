//! Writes every workflow in this crate as an upload payload (`*.workflow.json`).
//!
//! Usage: `cargo run -p namu-distributed-workflows --bin export -- [out_dir]`

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;

// Linking the library registers its workflows.
use namu_distributed_workflows as _;
use namu_proto::WorkflowUploadRequest;

const VERSION: &str = "0.1.0";

fn main() -> anyhow::Result<()> {
    let out_dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../dist/workflows"));
    std::fs::create_dir_all(&out_dir)?;

    for entry in namu_core::registry::get_workflows().values() {
        let ir = (entry.build)();
        let tasks: BTreeSet<String> = ir
            .operations
            .iter()
            .filter_map(|op| op.task_id().map(str::to_string))
            .collect();
        let payload = WorkflowUploadRequest {
            id: entry.id.to_string(),
            version: VERSION.to_string(),
            ir: serde_json::to_value(&ir)?,
            task_versions: tasks
                .into_iter()
                .map(|t| (t, VERSION.to_string()))
                .collect::<HashMap<_, _>>(),
        };
        let path = out_dir.join(format!("{}.workflow.json", entry.id));
        std::fs::write(&path, serde_json::to_string_pretty(&payload)?)?;
        println!("wrote {}", path.display());
    }
    Ok(())
}
