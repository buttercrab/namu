//! The control plane's rules, over the memory store, with a mesh that records what is sent.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use namu::prelude::*;
use namu_core::value::Value;
use namu_engine::RunOutcome;
use namu_engine::message::Message;
use namu_engine::run::WorkerStats;
use namu_proto::{
    Published, RunCreateRequest, RunStatus, TaskManifest, WorkerRegisterRequest,
    WorkflowUploadRequest,
};
use serde_json::json;
use sha2::Digest;
use uuid::Uuid;

use super::{Control, Mesh};
use crate::artifacts::{Artifacts, MemoryArtifacts};
use crate::error::ApiError;
use crate::store::{MemoryStore, Store};

#[task]
fn double(x: i32) -> Result<i32>;

#[workflow]
fn doubled(n: i32) -> i32 {
    double(n)
}

/// Remembers the messages sent, by worker.
#[derive(Default)]
struct Recorder {
    sent: Mutex<Vec<(u64, Message)>>,
}

impl Mesh for Recorder {
    fn connect(&self, _: u64, _: SocketAddr) {}

    fn disconnect(&self, _: u64) {}

    fn send(&self, worker: u64, message: Message) {
        self.sent.lock().unwrap().push((worker, message));
    }
}

impl Recorder {
    /// The runs started so far: (worker, run, inputs as JSON by value id).
    fn starts(&self) -> Vec<(u64, Uuid, Vec<serde_json::Value>)> {
        self.sent
            .lock()
            .unwrap()
            .iter()
            .filter_map(|(worker, message)| match message {
                Message::Start { run, inputs, .. } => {
                    let values = inputs.iter().map(|(_, value)| to_json(value)).collect();
                    Some((*worker, *run, values))
                }
                _ => None,
            })
            .collect()
    }
}

fn to_json(value: &Value) -> serde_json::Value {
    rmp_serde::from_slice(&value.encode().unwrap()).unwrap()
}

struct Plane {
    control: Control,
    store: Arc<MemoryStore>,
    artifacts: Arc<MemoryArtifacts>,
    mesh: Arc<Recorder>,
}

fn plane() -> Plane {
    let store = Arc::new(MemoryStore::default());
    let artifacts = Arc::new(MemoryArtifacts::default());
    let mesh = Arc::new(Recorder::default());
    let control = Control::new(
        store.clone(),
        artifacts.clone(),
        mesh.clone(),
        "127.0.0.1:7000".into(),
    );
    Plane {
        control,
        store,
        artifacts,
        mesh,
    }
}

fn manifest(id: &str, version: &str, code: &[u8]) -> TaskManifest {
    serde_json::from_value(json!({
        "task_id": id, "version": version, "task_kind": "single", "runtime": "wasm",
        "input_arity": 1, "output_arity": 1,
        "checksum": format!("sha256:{:x}", sha2::Sha256::digest(code)),
    }))
    .unwrap()
}

/// A task package, as `namu build` makes one.
fn package(manifest: &TaskManifest, code: &[u8]) -> Bytes {
    let mut tar = tar::Builder::new(Vec::new());
    for (name, bytes) in [
        ("manifest.json", serde_json::to_vec(manifest).unwrap()),
        ("task.wasm", code.to_vec()),
    ] {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        tar.append_data(&mut header, name, &bytes[..]).unwrap();
    }
    let tar = tar.into_inner().unwrap();
    Bytes::from(zstd::encode_all(&tar[..], 0).unwrap())
}

fn upload(version: &str, task_version: &str) -> WorkflowUploadRequest {
    WorkflowUploadRequest {
        id: "doubled".into(),
        version: version.into(),
        ir: serde_json::to_value(doubled().to_serializable("doubled".into())).unwrap(),
        task_versions: [("double".to_string(), task_version.to_string())].into(),
    }
}

fn worker(name: &str, port: u16) -> WorkerRegisterRequest {
    WorkerRegisterRequest {
        name: name.into(),
        addr: format!("127.0.0.1:{port}"),
        runtimes: vec!["wasm".into()],
        platform: "test".into(),
    }
}

fn run_of(inputs: serde_json::Value) -> RunCreateRequest {
    RunCreateRequest {
        workflow_id: "doubled".into(),
        version: "1".into(),
        inputs: inputs.as_object().unwrap().clone(),
    }
}

/// A plane with `doubled@1` published, and its task.
async fn published() -> Plane {
    let plane = plane();
    let code = b"\0asm double";
    let package = package(&manifest("double", "1", code), code);
    plane.control.publish_task(package).await.unwrap();
    plane
        .control
        .publish_workflow(upload("1", "1"))
        .await
        .unwrap();
    plane
}

#[tokio::test]
async fn a_task_version_is_published_once() {
    let plane = plane();
    let code = b"\0asm one";
    let first = package(&manifest("double", "1", code), code);
    let published = plane.control.publish_task(first.clone()).await.unwrap();
    assert_eq!(
        published,
        Published {
            id: "double".into(),
            version: "1".into(),
            unchanged: false
        }
    );
    assert_eq!(plane.control.artifact("double", "1").await.unwrap(), first);

    // Again, identically: nothing changes.
    assert!(
        plane
            .control
            .publish_task(first.clone())
            .await
            .unwrap()
            .unchanged
    );

    // Other code, or another manifest, under the same version: refused, and the first stays.
    let other = b"\0asm two";
    let second = package(&manifest("double", "1", other), other);
    let refused = plane.control.publish_task(second).await.unwrap_err();
    assert!(matches!(refused, ApiError::Conflict(_)), "{refused}");
    let mut batched = manifest("double", "1", code);
    batched.batch_size = Some(8);
    let refused = plane
        .control
        .publish_task(package(&batched, code))
        .await
        .unwrap_err();
    assert!(matches!(refused, ApiError::Conflict(_)), "{refused}");
    assert_eq!(plane.control.artifact("double", "1").await.unwrap(), first);
}

#[tokio::test]
async fn a_bad_package_is_refused() {
    let plane = plane();
    // Code that is not what the manifest's checksum says.
    let tampered = package(&manifest("double", "1", b"\0asm one"), b"\0asm evil");
    let refused = plane.control.publish_task(tampered).await.unwrap_err();
    assert!(refused.to_string().contains("checksum"), "{refused}");
    // An id that would collide with another in the store's keys.
    let code = b"\0asm";
    let colliding = package(&manifest("a:b", "1", code), code);
    let refused = plane.control.publish_task(colliding).await.unwrap_err();
    assert!(matches!(refused, ApiError::BadRequest(_)), "{refused}");
    let refused = plane.control.artifact("..", "1").await.unwrap_err();
    assert!(matches!(refused, ApiError::BadRequest(_)), "{refused}");
}

#[tokio::test]
async fn publishing_again_writes_an_artifact_a_crash_left_out() {
    let plane = plane();
    let code = b"\0asm one";
    let manifest = manifest("double", "1", code);
    // As if the orchestrator died after storing the manifest, before writing the artifact.
    plane.store.insert_task(&manifest).await.unwrap();
    assert!(plane.artifacts.get("double", "1").await.unwrap().is_none());

    let package = package(&manifest, code);
    assert!(
        plane
            .control
            .publish_task(package.clone())
            .await
            .unwrap()
            .unchanged
    );
    assert_eq!(
        plane.artifacts.get("double", "1").await.unwrap(),
        Some(package)
    );
}

#[tokio::test]
async fn a_workflow_is_published_once_its_tasks_are() {
    let plane = plane();
    let refused = plane
        .control
        .publish_workflow(upload("1", "1"))
        .await
        .unwrap_err();
    assert!(
        refused.to_string().contains("double@1 is not published"),
        "{refused}"
    );

    let code = b"\0asm double";
    let package = package(&manifest("double", "1", code), code);
    plane.control.publish_task(package).await.unwrap();
    assert!(
        !plane
            .control
            .publish_workflow(upload("1", "1"))
            .await
            .unwrap()
            .unchanged
    );
    assert!(
        plane
            .control
            .publish_workflow(upload("1", "1"))
            .await
            .unwrap()
            .unchanged
    );

    // The same version, calling another version of the task: refused.
    let refused = plane
        .control
        .publish_workflow(upload("1", "2"))
        .await
        .unwrap_err();
    assert!(matches!(refused, ApiError::BadRequest(_)), "{refused}");
    let mut changed = upload("1", "1");
    changed.ir["name"] = json!("renamed");
    let refused = plane.control.publish_workflow(changed).await.unwrap_err();
    assert!(matches!(refused, ApiError::Conflict(_)), "{refused}");

    let mut broken = upload("2", "1");
    broken.ir = json!({"name": "broken", "operations": "none"});
    let refused = plane.control.publish_workflow(broken).await.unwrap_err();
    assert!(matches!(refused, ApiError::BadRequest(_)), "{refused}");
}

#[tokio::test]
async fn runs_start_with_checked_inputs_on_workers_in_turn() {
    let plane = published().await;
    let refused = plane
        .control
        .start_run(run_of(json!({"n": 1})))
        .await
        .unwrap_err();
    assert!(matches!(refused, ApiError::Unavailable(_)), "{refused}");

    let one = plane.control.register(worker("w1", 7001)).await.unwrap();
    let two = plane.control.register(worker("w2", 7002)).await.unwrap();
    assert_eq!((one.worker_id, two.worker_id), (1, 2));

    let refused = plane
        .control
        .start_run(run_of(json!({"n": "one"})))
        .await
        .unwrap_err();
    assert!(
        refused.to_string().contains("`n`: expected i32"),
        "{refused}"
    );
    let refused = plane
        .control
        .start_run(run_of(json!({})))
        .await
        .unwrap_err();
    assert!(
        refused.to_string().contains("`n` (i32) is missing"),
        "{refused}"
    );
    let mut unknown = run_of(json!({"n": 1}));
    unknown.version = "9".into();
    let refused = plane.control.start_run(unknown).await.unwrap_err();
    assert!(matches!(refused, ApiError::NotFound(_)), "{refused}");
    assert!(plane.mesh.starts().is_empty());

    let first = plane
        .control
        .start_run(run_of(json!({"n": 3})))
        .await
        .unwrap();
    let second = plane
        .control
        .start_run(run_of(json!({"n": 4})))
        .await
        .unwrap();
    let starts = plane.mesh.starts();
    let owners: Vec<u64> = starts.iter().map(|(worker, ..)| *worker).collect();
    assert_eq!(owners.len(), 2);
    assert_ne!(owners[0], owners[1], "owners take turns");
    assert_eq!(starts[0].1, first);
    assert_eq!(starts[0].2, [json!(3)]);
    assert_eq!(starts[1].1, second);
    assert_eq!(starts[1].2, [json!(4)]);

    let status = plane.control.run_status(first).await.unwrap();
    assert_eq!(status.status, RunStatus::Running);

    // A draining worker owns no new run.
    plane.control.drain(1).await.unwrap();
    for n in 0..4 {
        plane
            .control
            .start_run(run_of(json!({"n": n})))
            .await
            .unwrap();
    }
    assert!(
        plane.mesh.starts()[2..]
            .iter()
            .all(|(worker, ..)| *worker == 2)
    );
}

#[tokio::test]
async fn a_run_ends_with_its_owners_report() {
    let plane = published().await;
    plane.control.register(worker("w1", 7001)).await.unwrap();
    let run = plane
        .control
        .start_run(run_of(json!({"n": 21})))
        .await
        .unwrap();

    let outcome = RunOutcome {
        results: vec![Value::new(42i32)],
        failures: Vec::new(),
        workers: BTreeMap::from([(
            1,
            WorkerStats {
                calls: 1,
                fetches: 0,
                ..WorkerStats::default()
            },
        )]),
    };
    plane.control.finished(run, outcome).await.unwrap();
    let status = plane.control.run_status(run).await.unwrap();
    assert_eq!(status.status, RunStatus::Succeeded);
    assert_eq!(status.results, [json!(42)]);
    assert_eq!(
        status.calls_by_worker,
        BTreeMap::from([("w1".to_string(), 1)])
    );
    assert!(plane.store.running().await.unwrap().is_empty());

    let run = plane
        .control
        .start_run(run_of(json!({"n": 1})))
        .await
        .unwrap();
    let outcome = RunOutcome {
        failures: vec!["a branch failed".into()],
        ..RunOutcome::default()
    };
    plane.control.finished(run, outcome).await.unwrap();
    let status = plane.control.run_status(run).await.unwrap();
    assert_eq!(status.status, RunStatus::PartialFailed);
    assert_eq!(status.failures, ["a branch failed"]);

    let refused = plane.control.run_status(Uuid::new_v4()).await.unwrap_err();
    assert!(matches!(refused, ApiError::NotFound(_)), "{refused}");
}

#[tokio::test]
async fn a_dead_owners_runs_start_again_on_its_successor() {
    let plane = published().await;
    for (name, port) in [("w1", 7001), ("w2", 7002), ("w3", 7003)] {
        plane.control.register(worker(name, port)).await.unwrap();
    }
    let runs: Vec<Uuid> = start_runs(&plane, 3).await;
    let owned_by_two: Vec<Uuid> = plane
        .mesh
        .starts()
        .into_iter()
        .filter(|(worker, ..)| *worker == 2)
        .map(|(_, run, _)| run)
        .collect();
    assert_eq!(owned_by_two.len(), 1, "one run each: {runs:?}");

    // Worker 2's lease ends; the lowest live worker succeeds it.
    plane.store.end_lease(2).await.unwrap();
    plane.control.sweep().await.unwrap();
    let membership = plane.control.membership().await.unwrap();
    assert_eq!(membership.successors, BTreeMap::from([(2, 1)]));
    let ids: Vec<u64> = membership.members.iter().map(|m| m.id).collect();
    assert_eq!(ids, [0, 1, 3]);

    // Its run starts again on worker 1, with the inputs it was given.
    let again: Vec<_> = plane.mesh.starts()[3..].to_vec();
    assert_eq!(again.len(), 1);
    assert_eq!((again[0].0, again[0].1), (1, owned_by_two[0]));
    let record = plane.store.run(owned_by_two[0]).await.unwrap().unwrap();
    assert_eq!(record.owner, 1);
    assert_eq!(again[0].2, [record.spec.inputs["n"].clone()]);

    // A second sweep changes nothing.
    plane.control.sweep().await.unwrap();
    assert_eq!(plane.mesh.starts().len(), 4);

    // The dead worker is no longer a member: it must register again.
    let refused = plane.control.heartbeat(2).await.unwrap_err();
    assert!(matches!(refused, ApiError::Gone(_)), "{refused}");
    let refused = plane.control.drain(2).await.unwrap_err();
    assert!(matches!(refused, ApiError::Gone(_)), "{refused}");
    assert_eq!(plane.control.heartbeat(1).await.unwrap().members.len(), 3);
}

/// Starts `n` runs of `doubled`, with `n` = 0, 1, ...
async fn start_runs(plane: &Plane, n: i32) -> Vec<Uuid> {
    let mut runs = Vec::new();
    for i in 0..n {
        runs.push(
            plane
                .control
                .start_run(run_of(json!({"n": i})))
                .await
                .unwrap(),
        );
    }
    runs
}
