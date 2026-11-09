//! The control plane's rules, over the memory store, with a mesh that records what is sent.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use namu::prelude::*;
use namu_core::ir::Operation;
use namu_core::signature::Signature;
use namu_core::types::{Defs, Type};
use namu_core::value::Value;
use namu_engine::message::{BranchId, Delivery, Message, Report};
use namu_engine::run::WorkerStats;
use namu_proto::{
    PageQuery, Published, RunCreateRequest, RunStatus, TaskManifest, WorkerRegisterRequest,
    WorkflowUploadRequest,
};
use serde_json::json;
use sha2::Digest;
use uuid::Uuid;

use super::{Control, Mesh};
use crate::artifacts::Artifacts;
use crate::error::ApiError;
use crate::store::{MemoryStore, Store};

#[task]
fn double(x: i32) -> Result<i32>;

#[workflow]
fn doubled(n: i32) -> i32 {
    double(n)
}

#[task]
fn items(n: i32) -> Result<Vec<i32>>;

#[task]
fn load(n: i32) -> Result<i32>;

#[task]
fn add(a: i32, b: i32) -> Result<i32>;

#[task]
fn sum(xs: Vec<i32>) -> Result<i32>;

#[workflow]
fn labelled(n: i32) -> i32 {
    let item = each(items(n));
    let model = load(n); // every branch alike
    sum(add(model, item))
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
    artifacts: Arc<Artifacts>,
    mesh: Arc<Recorder>,
}

fn plane() -> Plane {
    let store = Arc::new(MemoryStore::default());
    let artifacts = Arc::new(Artifacts::in_memory());
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
async fn a_workflow_is_checked_against_its_tasks_signatures() {
    let plane = plane();
    let code = b"\0asm double";
    let int = Type::Int {
        bits: 64,
        signed: true,
    };
    let publish = |version: &'static str, signature: Option<Signature>| {
        let mut manifest = manifest("double", version, code);
        manifest.signature = signature;
        plane.control.publish_task(package(&manifest, code))
    };
    // `double` as built three times: taking a string, taking an integer, and saying nothing.
    let takes = |input: Type| Some(Signature::new(vec![input], int.clone(), Defs::new()));
    publish("text", takes(Type::String)).await.unwrap();
    publish("int", takes(int.clone())).await.unwrap();
    publish("old", None).await.unwrap();

    let refused = plane
        .control
        .publish_workflow(upload("1", "text"))
        .await
        .unwrap_err();
    assert_eq!(
        refused.to_string(),
        ApiError::bad_request(
            "workflow doubled@1 does not fit its tasks:\n\
             call to `double` (op 0): argument 1: `i32` is read as `String`"
        )
        .to_string()
    );
    let fits = plane
        .control
        .publish_workflow(upload("2", "int"))
        .await
        .unwrap();
    assert!(fits.unchecked.is_empty());
    let unchecked = plane
        .control
        .publish_workflow(upload("3", "old"))
        .await
        .unwrap();
    assert_eq!(unchecked.unchecked, ["double"]);
}

#[tokio::test]
async fn runs_make_once_what_every_branch_makes_alike() {
    let plane = plane();
    let code = b"\0asm";
    for task in ["items", "add", "sum"] {
        let manifest = manifest(task, "1", code);
        plane
            .control
            .publish_task(package(&manifest, code))
            .await
            .unwrap();
    }
    // `load` twice: as a plain task, and as one with side effects.
    let mut effects = manifest("load", "effects", code);
    effects.checkpoint = true;
    for load in [manifest("load", "pure", code), effects] {
        plane
            .control
            .publish_task(package(&load, code))
            .await
            .unwrap();
    }
    let ir = serde_json::to_value(labelled().to_serializable("labelled".into())).unwrap();
    // Where `load` is called, and where the items fan out, in the workflow runs start.
    let order = async |load: &str| {
        let task_versions = [("items", "1"), ("add", "1"), ("sum", "1"), ("load", load)];
        let upload = WorkflowUploadRequest {
            id: "labelled".into(),
            version: load.into(),
            ir: ir.clone(),
            task_versions: task_versions
                .into_iter()
                .map(|(task, version)| (task.to_string(), version.to_string()))
                .collect(),
        };
        plane.control.publish_workflow(upload).await.unwrap();
        let version = plane
            .control
            .version("labelled", load)
            .await
            .unwrap()
            .unwrap();
        let ops = &version.workflow.operations;
        let at = |found: fn(&Operation) -> bool| ops.iter().position(found).unwrap();
        (
            at(|op| op.task_id() == Some("load")),
            at(|op| op.fan_out().is_some()),
        )
    };
    let (load, fan_out) = order("pure").await;
    assert!(load < fan_out, "made once, before the fan-out");
    let (load, fan_out) = order("effects").await;
    assert!(load > fan_out, "made by every branch");
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
            unchanged: false,
            unchecked: Vec::new(),
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

fn branch(index: u64) -> BranchId {
    BranchId { scope: 7, index }
}

fn first(query_after: u64, limit: u64) -> PageQuery {
    PageQuery {
        after: query_after,
        limit: Some(limit),
        wait_ms: None,
    }
}

/// Delivery `seq` from worker 1.
fn delivery(seq: u64) -> Delivery {
    Delivery { from: 1, seq }
}

#[tokio::test]
async fn reports_are_stored_each_once_and_their_owner_told() {
    let plane = published().await;
    plane.control.register(worker("w1", 7001)).await.unwrap();
    let run = plane
        .control
        .start_run(run_of(json!({"n": 21})))
        .await
        .unwrap();

    for i in 0..5 {
        let report = Report::Result(Value::new(i * 10));
        let branch = branch(i as u64);
        plane
            .control
            .recorded(run, branch, report, delivery(i as u64))
            .await
            .unwrap();
    }
    // Sent again by a successor: the same branches, counted once, and acknowledged again.
    for i in 0..5 {
        let report = Report::Result(Value::new(i * 10));
        let branch = branch(i as u64);
        plane
            .control
            .recorded(run, branch, report, delivery(10 + i as u64))
            .await
            .unwrap();
    }
    let failure = Report::Failure("a branch failed".into());
    plane
        .control
        .recorded(run, branch(5), failure, delivery(20))
        .await
        .unwrap();
    // Every one is acknowledged to the worker that sent it, once stored.
    let stored: Vec<u64> = plane
        .mesh
        .sent
        .lock()
        .unwrap()
        .iter()
        .filter_map(|(worker, message)| match message {
            Message::Stored { seqs } if *worker == 1 => Some(seqs.clone()),
            _ => None,
        })
        .flatten()
        .collect();
    assert_eq!(stored, [0, 1, 2, 3, 4, 10, 11, 12, 13, 14, 20]);

    let status = plane.control.run_status(run).await.unwrap();
    assert_eq!(status.status, RunStatus::Running);
    assert_eq!((status.results, status.failures), (5, 1));

    // Read in pages, in the order they arrived, while the run goes on.
    let page = plane.control.results(run, first(0, 2)).await.unwrap();
    assert_eq!((page.items, page.next), (vec![json!(0), json!(10)], 2));
    assert_eq!(page.status, RunStatus::Running);
    let page = plane.control.results(run, first(2, 10)).await.unwrap();
    assert_eq!(
        (page.items, page.next),
        (vec![json!(20), json!(30), json!(40)], 5)
    );
    let page = plane.control.results(run, first(5, 10)).await.unwrap();
    assert_eq!((page.items.len(), page.next), (0, 5));
    let failures = plane
        .control
        .failures(run, PageQuery::default())
        .await
        .unwrap();
    assert_eq!(failures.items, ["a branch failed"]);

    let stats = WorkerStats {
        calls: 6,
        ..WorkerStats::default()
    };
    plane
        .control
        .finished(run, BTreeMap::from([(1, stats)]))
        .await
        .unwrap();
    let status = plane.control.run_status(run).await.unwrap();
    assert_eq!(status.status, RunStatus::PartialFailed);
    assert_eq!((status.results, status.failures), (5, 1));
    assert_eq!(
        status.calls_by_worker,
        BTreeMap::from([("w1".to_string(), 6)])
    );
    assert!(plane.store.running().await.unwrap().is_empty());

    // A report arriving after the end was stored before it; it is acknowledged, not stored again.
    let late = Report::Result(Value::new(0));
    plane
        .control
        .recorded(run, branch(0), late, delivery(30))
        .await
        .unwrap();
    assert_eq!(plane.control.run_status(run).await.unwrap().results, 5);
    let page = plane
        .control
        .results(run, PageQuery::default())
        .await
        .unwrap();
    assert_eq!(page.status, RunStatus::PartialFailed);
    assert_eq!(page.items.len(), 5);

    let refused = plane.control.run_status(Uuid::new_v4()).await.unwrap_err();
    assert!(matches!(refused, ApiError::NotFound(_)), "{refused}");
    let refused = plane
        .control
        .results(Uuid::new_v4(), PageQuery::default())
        .await
        .unwrap_err();
    assert!(matches!(refused, ApiError::NotFound(_)), "{refused}");
}

#[tokio::test]
async fn a_run_without_failures_succeeds() {
    let plane = published().await;
    plane.control.register(worker("w1", 7001)).await.unwrap();
    let run = plane
        .control
        .start_run(run_of(json!({"n": 21})))
        .await
        .unwrap();
    let report = Report::Result(Value::new(42));
    plane
        .control
        .recorded(run, branch(0), report, delivery(0))
        .await
        .unwrap();
    plane.control.finished(run, BTreeMap::new()).await.unwrap();
    let status = plane.control.run_status(run).await.unwrap();
    assert_eq!(status.status, RunStatus::Succeeded);
    let page = plane
        .control
        .results(run, PageQuery::default())
        .await
        .unwrap();
    assert_eq!(page.items, [json!(42)]);
}

#[tokio::test]
async fn a_follower_waits_for_the_next_report_or_the_end() {
    let plane = published().await;
    plane.control.register(worker("w1", 7001)).await.unwrap();
    let run = plane
        .control
        .start_run(run_of(json!({"n": 21})))
        .await
        .unwrap();
    let waiting = PageQuery {
        wait_ms: Some(10_000),
        ..PageQuery::default()
    };
    let started = std::time::Instant::now();

    // Nothing yet: the page waits, and holds the report as soon as it is stored.
    let (page, ()) = tokio::join!(plane.control.results(run, waiting), async {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let report = Report::Result(Value::new(7));
        plane
            .control
            .recorded(run, branch(0), report, delivery(0))
            .await
            .unwrap();
    });
    assert_eq!(page.unwrap().items, [json!(7)]);

    // Past it, the page waits for the run's end, and says so.
    let after = PageQuery {
        after: 1,
        ..waiting
    };
    let (page, ()) = tokio::join!(plane.control.results(run, after), async {
        tokio::time::sleep(Duration::from_millis(50)).await;
        plane.control.finished(run, BTreeMap::new()).await.unwrap();
    });
    let page = page.unwrap();
    assert!(page.items.is_empty());
    assert_eq!(page.status, RunStatus::Succeeded);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "woken, not timed out"
    );

    // Over, a run's page does not wait.
    let page = plane.control.failures(run, after).await.unwrap();
    assert_eq!(page.status, RunStatus::Succeeded);
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

    // Worker 2's lease ends. Until the sweep buries it, it is still a member: a lease may run
    // out only because the store was down, and peers that dropped it would never take it back.
    plane.store.end_lease(2).await.unwrap();
    let ids: Vec<u64> = (plane.control.membership().await.unwrap().members)
        .iter()
        .map(|m| m.id)
        .collect();
    assert_eq!(ids, [0, 1, 2, 3]);

    // The sweep buries it, and the lowest live worker succeeds it.
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

/// A control plane that restarted knows its runs from the store, not whether their owners got
/// them: it sends each that has not ended its `Start` again, to its owner or its owner's heir.
#[tokio::test]
async fn a_restarted_control_plane_starts_its_runs_again() {
    let before = published().await;
    for (name, port) in [("w1", 7001), ("w2", 7002)] {
        before.control.register(worker(name, port)).await.unwrap();
    }
    let runs = start_runs(&before, 2).await;
    before
        .control
        .finished(runs[0], BTreeMap::new())
        .await
        .unwrap();
    // Worker 2 died, and the old control plane named worker 1 its successor.
    before.store.end_lease(2).await.unwrap();
    before.control.sweep().await.unwrap();

    // A new control plane, on the same store.
    let mesh = Arc::new(Recorder::default());
    let after = Control::new(
        before.store.clone(),
        before.artifacts.clone(),
        mesh.clone(),
        "127.0.0.1:7000".into(),
    );
    after.recover().await.unwrap();
    let starts = mesh.starts();
    // Only the run that has not ended, on a live worker, with the inputs it was given.
    assert_eq!(starts.len(), 1, "{starts:?}");
    let (worker, run, inputs) = &starts[0];
    assert_eq!((*worker, *run), (1, runs[1]));
    assert_eq!(inputs, &[json!(1)]);
    assert_eq!(before.store.run(runs[1]).await.unwrap().unwrap().owner, 1);
}

/// While the control plane is down, nobody can renew a lease. A restart that took longer than a
/// lease must not bury every worker: leases that ran out unrenewed are renewed on the way up.
#[tokio::test]
async fn a_long_restart_buries_nobody() {
    let before = published().await;
    for (name, port) in [("w1", 7001), ("w2", 7002)] {
        before.control.register(worker(name, port)).await.unwrap();
    }
    // Down for longer than a lease: every lease ran out.
    for id in [1, 2] {
        before.store.end_lease(id).await.unwrap();
    }
    let after = Control::new(
        before.store.clone(),
        before.artifacts.clone(),
        Arc::new(Recorder::default()),
        "127.0.0.1:7000".into(),
    );
    after.recover().await.unwrap();
    after.sweep().await.unwrap();
    let membership = after.membership().await.unwrap();
    let ids: Vec<u64> = membership.members.iter().map(|m| m.id).collect();
    assert_eq!(ids, [0, 1, 2]);
    assert!(membership.successors.is_empty());
    // And their heartbeats go on.
    assert!(after.heartbeat(1).await.is_ok());
}
