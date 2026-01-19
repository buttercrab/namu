use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use namu_proto::WorkflowUploadRequest;

fn task_versions() -> HashMap<String, String> {
    let mut versions = HashMap::new();
    for task in [
        "add",
        "less_than",
        "is_even",
        "id_range",
        "normalize",
        "embed_batch",
        "score",
        "maybe_fail",
    ] {
        versions.insert(task.to_string(), "0.1.0".to_string());
    }
    versions
}

fn out_dir_from_args() -> PathBuf {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--out"
            && let Some(path) = args.next()
        {
            return PathBuf::from(path);
        }
    }

    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    base.join("../dist/workflows")
}

fn write_workflow(path: &Path, id: &str, graph: namu_flow::Graph<i32>) {
    let wf_ir = graph.to_serializable(id.to_string());
    let payload = WorkflowUploadRequest {
        id: id.to_string(),
        version: "0.1.0".to_string(),
        ir: serde_json::to_value(wf_ir).expect("serialize workflow"),
        task_versions: task_versions(),
    };

    let json = serde_json::to_string_pretty(&payload).expect("encode workflow json");
    let file_path = path.join(format!("{id}.workflow.json"));
    fs::write(&file_path, json).expect("write workflow json");
    println!("Wrote {}", file_path.display());
}

fn main() {
    let out_dir = out_dir_from_args();
    fs::create_dir_all(&out_dir).expect("create workflow output dir");

    write_workflow(
        &out_dir,
        "etl_pipeline",
        namu_advanced_workflows::etl_pipeline(),
    );
    write_workflow(
        &out_dir,
        "ml_pipeline",
        namu_advanced_workflows::ml_pipeline(),
    );
    write_workflow(
        &out_dir,
        "media_pipeline",
        namu_advanced_workflows::media_pipeline(),
    );
}
