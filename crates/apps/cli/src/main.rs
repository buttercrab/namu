mod config;
mod exporter;
mod signature;
mod sync;

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Context;
use clap::{Parser, Subcommand};
use namu_proto::{
    Page, PageQuery, Published, RunCreateRequest, RunCreateResponse, RunStatus, TaskManifest,
    TaskRuntime, WorkflowUploadRequest,
};
use reqwest::multipart;
use serde_json::Value as JsonValue;
use sha2::Digest;
use walkdir::WalkDir;

#[derive(Parser)]
#[command(name = "namu")]
#[command(about = "NAMU pipeline engine CLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Build task artifacts and workflow IR bundles
    Build {
        /// Path to tasks directory (default: ./tasks)
        #[arg(short, long, default_value = "./tasks")]
        tasks_dir: PathBuf,
        /// Path to workflows directory (default: ./workflows)
        #[arg(short, long, default_value = "./workflows")]
        workflows_dir: PathBuf,
        /// Output directory (default: ./dist)
        #[arg(short, long, default_value = "./dist")]
        out_dir: PathBuf,
        /// Path to namu.toml (default: ./namu.toml if present)
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Sync namu.toml into Cargo.toml and registry config
    Sync {
        /// Path to namu.toml (default: ./namu.toml)
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Publish built artifacts and workflows to orchestrator
    Publish {
        /// Output directory used by build (default: ./dist)
        #[arg(short, long, default_value = "./dist")]
        out_dir: PathBuf,
    },
    /// Create a run
    Run {
        workflow_id: String,
        version: String,
        /// A value for a workflow parameter, `name=value`; repeat for each. The value is JSON
        /// (`limit=300`, `tags=["a","b"]`), or else a string (`collection=wikipedia`).
        #[arg(long = "input", value_name = "NAME=VALUE", value_parser = parse_input)]
        inputs: Vec<(String, serde_json::Value)>,
        /// Wait for the run to finish and print its status, results and failures
        #[arg(long)]
        wait: bool,
    },
    /// Check run status
    Status { run_id: String },
    /// Print a run's results, one JSON value per line, in the order they arrived
    Results {
        run_id: String,
        /// Keep printing them as they arrive, until the run ends
        #[arg(long)]
        follow: bool,
        /// The failures instead
        #[arg(long)]
        failures: bool,
    },
    /// List workers
    Workers,
    /// Login and healthcheck orchestrator
    Login,
    /// Show version information
    Version,
    /// Make an API token: prints it, and adds its hash to the orchestrator's tokens file
    Token {
        /// `client` (publish, run, read results) or `worker` (join the mesh)
        #[arg(long)]
        role: String,
        /// The orchestrator's tokens file (`NAMU_TOKENS`); without it, the line is printed
        #[arg(long)]
        tokens: Option<PathBuf>,
    },
    /// Make a cluster CA and a member certificate, for development: a production cluster gets
    /// them from its own PKI
    Certs {
        /// Where to write ca.pem, member.pem and member.key
        #[arg(long)]
        out: PathBuf,
        /// A name or address the orchestrator's HTTPS is reached by; repeat for each
        #[arg(long = "host")]
        hosts: Vec<String>,
    },
}

#[tokio::main]
async fn main() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let cli = Cli::parse();

    match cli.command {
        Commands::Build {
            tasks_dir,
            workflows_dir,
            out_dir,
            config,
        } => {
            if let Err(err) = build(&tasks_dir, &workflows_dir, &out_dir, config) {
                eprintln!("Build failed: {err}");
                std::process::exit(1);
            }
        }
        Commands::Sync { config } => {
            if let Err(err) = sync_config(config) {
                eprintln!("Sync failed: {err}");
                std::process::exit(1);
            }
        }
        Commands::Publish { out_dir } => {
            if !publish(&out_dir).await {
                std::process::exit(1);
            }
        }
        Commands::Run {
            workflow_id,
            version,
            inputs,
            wait,
        } => {
            run_workflow(&workflow_id, &version, inputs.into_iter().collect(), wait).await;
        }
        Commands::Status { run_id } => {
            run_status(&run_id).await;
        }
        Commands::Results {
            run_id,
            follow,
            failures,
        } => {
            let what = if failures { "failures" } else { "results" };
            if let Err(err) = print_results(&run_id, what, follow).await {
                eprintln!("Failed to fetch {what}: {err:#}");
                std::process::exit(1);
            }
        }
        Commands::Workers => {
            list_workers().await;
        }
        Commands::Login => {
            login().await;
        }
        Commands::Version => {
            show_version();
        }
        Commands::Token { role, tokens } => {
            if let Err(err) = make_token(&role, tokens.as_deref()) {
                eprintln!("Error: {err:#}");
                std::process::exit(1);
            }
        }
        Commands::Certs { out, hosts } => {
            if let Err(err) = make_certs(&out, &hosts) {
                eprintln!("Error: {err:#}");
                std::process::exit(1);
            }
        }
    }
}

fn build(
    tasks_dir: &Path,
    workflows_dir: &Path,
    out_dir: &Path,
    config_path: Option<PathBuf>,
) -> anyhow::Result<()> {
    if let Some(path) = config::find_config(config_path) {
        let cfg = config::load_config(&path)?;
        return build_from_config(&cfg);
    }

    fs::create_dir_all(out_dir.join("tasks"))?;
    fs::create_dir_all(out_dir.join("workflows"))?;

    build_tasks(tasks_dir, &out_dir.join("tasks"))?;
    build_workflows(workflows_dir, &out_dir.join("workflows"))?;
    Ok(())
}

fn sync_config(config_path: Option<PathBuf>) -> anyhow::Result<()> {
    let path =
        config::find_config(config_path).ok_or_else(|| anyhow::anyhow!("namu.toml not found"))?;
    let cfg = config::load_config(&path)?;
    sync::sync_config(&cfg)?;
    Ok(())
}

fn build_from_config(cfg: &config::NamuConfig) -> anyhow::Result<()> {
    let out_dir = &cfg.build.out_dir;
    fs::create_dir_all(out_dir.join("tasks"))?;
    fs::create_dir_all(out_dir.join("workflows"))?;

    build_tasks_from_config(cfg, &out_dir.join("tasks"))?;
    build_workflows_from_config(cfg, &out_dir.join("workflows"))?;
    Ok(())
}

fn build_tasks(tasks_dir: &Path, out_dir: &Path) -> anyhow::Result<()> {
    for entry in WalkDir::new(tasks_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name() == "manifest.json")
    {
        let task_dir = entry.path().parent().unwrap();
        let manifest: TaskManifest = serde_json::from_str(&fs::read_to_string(entry.path())?)
            .with_context(|| format!("reading {}", entry.path().display()))?;
        build_task(task_dir, manifest, out_dir)?;
    }
    Ok(())
}

fn build_tasks_from_config(cfg: &config::NamuConfig, out_dir: &Path) -> anyhow::Result<()> {
    for task in cfg.tasks.values() {
        let task_dir = task
            .path
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("task {} missing path", task.id))?;
        build_task(task_dir, task_manifest_from_config(task), out_dir)?;
    }
    Ok(())
}

/// Builds the task in `task_dir` (a crate is compiled; a Python module is taken as it is), records
/// its checksum and signature in `manifest`, and packages both in `out_dir`.
fn build_task(task_dir: &Path, mut manifest: TaskManifest, out_dir: &Path) -> anyhow::Result<()> {
    let id = format!("{}@{}", manifest.task_id, manifest.version);
    let crate_dir = task_dir.join("Cargo.toml").exists().then_some(task_dir);
    if let Some(task_dir) = crate_dir {
        let target = (manifest.runtime == TaskRuntime::Wasm).then_some(WASM_TARGET);
        cargo_build(task_dir, target)?;
    }

    let artifact_path = find_artifact(task_dir, &manifest.runtime)?;
    let artifact_bytes = fs::read(&artifact_path)?;
    let checksum = sha2::Sha256::digest(&artifact_bytes);
    manifest.checksum = format!("sha256:{:x}", checksum);
    set_build_info(&mut manifest);
    manifest.signature = match (manifest.runtime, crate_dir) {
        (TaskRuntime::Native, Some(_)) => Some(signature::native(&artifact_path)?),
        // The same crate, built for this machine, says what the component takes.
        (TaskRuntime::Wasm, Some(task_dir)) => {
            cargo_build(task_dir, None)?;
            Some(signature::native(&find_library(task_dir)?)?)
        }
        (TaskRuntime::Python, _) => signature::python(&artifact_path, &manifest.task_id)?,
        (_, None) => None,
    };
    if manifest.signature.is_none() {
        eprintln!("warning: no signature recorded for {id}; calls of it will not be checked");
    }

    let tar_path = out_dir.join(format!("{}-{}.tar.zst", manifest.task_id, manifest.version));
    package_task(&tar_path, &manifest, &artifact_path)?;
    println!("Built {}", tar_path.display());
    Ok(())
}

/// `cargo build --release` of the crate in `task_dir`, for `target` or this machine.
fn cargo_build(task_dir: &Path, target: Option<&str>) -> anyhow::Result<()> {
    let mut cmd = Command::new("cargo");
    cmd.args(["build", "--release", "--manifest-path"])
        .arg(task_dir.join("Cargo.toml"));
    if let Some(target) = target {
        cmd.args(["--target", target]);
    }
    let status = cmd.status()?;
    anyhow::ensure!(
        status.success(),
        "cargo build failed for {}",
        task_dir.display()
    );
    Ok(())
}

const WASM_TARGET: &str = "wasm32-wasip2";

/// Native artifacts only run on the platform they were built for, so record it; WASM runs
/// anywhere.
fn set_build_info(manifest: &mut TaskManifest) {
    manifest.platform = match manifest.runtime {
        TaskRuntime::Native => Some(namu_proto::host_platform()),
        TaskRuntime::Wasm | TaskRuntime::Python => None,
    };
    manifest.abi_version = namu_proto::TASK_ABI_VERSION.to_string();
}

fn task_manifest_from_config(task: &config::TaskConfig) -> TaskManifest {
    TaskManifest {
        task_id: task.id.clone(),
        version: task.version.clone(),
        task_kind: task.kind.clone(),
        trust: task.trust.clone(),
        runtime: task.runtime,
        platform: None,
        checkpoint: task.checkpoint,
        retries: task.retries,
        retry_backoff_ms: task.retry_backoff_ms,
        window: task.window,
        batch_size: task.batch_size,
        max_wait_ms: task.max_wait_ms,
        batch_slots: task.batch_slots,
        requires_gpu: task.requires_gpu,
        resource_class: task.resource_class.clone(),
        capabilities: task.capabilities.clone(),
        signature: None,
        checksum: String::new(),
        abi_version: namu_proto::TASK_ABI_VERSION.to_string(),
        build_toolchain: "unknown".to_string(),
        created_at: "unknown".to_string(),
    }
}

fn find_artifact(task_dir: &Path, runtime: &TaskRuntime) -> anyhow::Result<PathBuf> {
    match runtime {
        TaskRuntime::Native => find_library(task_dir),
        TaskRuntime::Wasm => find_wasm(task_dir),
        TaskRuntime::Python => {
            find_single_artifact(task_dir, |name| TaskRuntime::Python.is_artifact(name)).ok_or_else(
                || anyhow::anyhow!("expected 1 Python module in {}", task_dir.display()),
            )
        }
    }
}

fn find_library(task_dir: &Path) -> anyhow::Result<PathBuf> {
    let target_dir = task_dir.join("target/release");
    find_single_artifact(&target_dir, |name| TaskRuntime::Native.is_artifact(name))
        .ok_or_else(|| anyhow::anyhow!("expected 1 library in {}", target_dir.display()))
}

fn find_wasm(task_dir: &Path) -> anyhow::Result<PathBuf> {
    let dir = task_dir.join("target").join(WASM_TARGET).join("release");
    find_single_artifact(&dir, |name| TaskRuntime::Wasm.is_artifact(name))
        .ok_or_else(|| anyhow::anyhow!("expected 1 wasm component in {}", dir.display()))
}

fn package_task(
    path: &Path,
    manifest: &namu_proto::TaskManifest,
    artifact_path: &Path,
) -> anyhow::Result<()> {
    let mut encoder = zstd::Encoder::new(fs::File::create(path)?, 3)?;
    {
        let mut tar = tar::Builder::new(&mut encoder);

        let manifest_json = serde_json::to_vec_pretty(manifest)?;
        let mut header = tar::Header::new_gnu();
        header.set_size(manifest_json.len() as u64);
        header.set_cksum();
        tar.append_data(&mut header, "manifest.json", manifest_json.as_slice())?;

        let lib_bytes = fs::read(artifact_path)?;
        let mut header = tar::Header::new_gnu();
        header.set_size(lib_bytes.len() as u64);
        header.set_cksum();
        let lib_name = artifact_path.file_name().unwrap().to_string_lossy();
        tar.append_data(&mut header, lib_name.as_ref(), lib_bytes.as_slice())?;

        tar.finish()?;
    }
    encoder.finish()?;
    Ok(())
}

fn find_single_artifact(dir: &Path, matcher: impl Fn(&str) -> bool) -> Option<PathBuf> {
    if !dir.exists() {
        return None;
    }
    let mut candidates = Vec::new();
    for entry in fs::read_dir(dir).ok()? {
        let entry = entry.ok()?;
        let path = entry.path();
        let name = path.file_name().unwrap().to_string_lossy();
        if matcher(&name) {
            candidates.push(path);
        }
    }
    if candidates.len() == 1 {
        Some(candidates.remove(0))
    } else {
        None
    }
}

fn build_workflows(workflows_dir: &Path, out_dir: &Path) -> anyhow::Result<()> {
    if !workflows_dir.exists() {
        return Ok(());
    }
    for entry in WalkDir::new(workflows_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| e.file_name().to_string_lossy().ends_with(".workflow.json"))
    {
        let out_path = out_dir.join(entry.file_name());
        fs::copy(entry.path(), &out_path)?;
        println!("Copied workflow {}", out_path.display());
    }
    Ok(())
}

fn build_workflows_from_config(cfg: &config::NamuConfig, out_dir: &Path) -> anyhow::Result<()> {
    let ir_dir = exporter::export_workflows(cfg)?;
    let mut written = 0;

    for entry in fs::read_dir(&ir_dir)? {
        let entry = entry?;
        let path = entry.path();
        let file_name = path.file_name().and_then(|n| n.to_str());
        if file_name
            .map(|n| !n.ends_with(".workflow.ir.json"))
            .unwrap_or(true)
        {
            continue;
        }

        let raw = fs::read_to_string(&path)?;
        let workflow: namu_core::ir::Workflow = serde_json::from_str(&raw)?;
        let id = workflow.name.clone();

        let wf_cfg = cfg
            .workflows
            .entries
            .get(&id)
            .ok_or_else(|| anyhow::anyhow!("missing workflows.{id}.version"))?;

        let task_versions = task_versions_for_workflow(cfg, &workflow)?;

        let payload = WorkflowUploadRequest {
            id: id.clone(),
            version: wf_cfg.version.clone(),
            ir: serde_json::to_value(workflow)?,
            task_versions,
        };

        let json = serde_json::to_string_pretty(&payload)?;
        let file_path = out_dir.join(format!("{id}.workflow.json"));
        fs::write(&file_path, json)?;
        written += 1;
    }

    if written == 0 {
        return Err(anyhow::anyhow!("no workflows exported"));
    }

    Ok(())
}

fn task_versions_for_workflow(
    cfg: &config::NamuConfig,
    workflow: &namu_core::ir::Workflow,
) -> anyhow::Result<std::collections::HashMap<String, String>> {
    let mut task_ids = std::collections::BTreeSet::new();
    for op in &workflow.operations {
        if let Some(call) = op.call() {
            task_ids.insert(call.task_id.clone());
        }
    }

    let mut versions = std::collections::HashMap::new();
    for id in task_ids {
        let task = cfg
            .tasks
            .get(&id)
            .ok_or_else(|| anyhow::anyhow!("task {id} missing from namu.toml"))?;
        versions.insert(id, task.version.clone());
    }

    Ok(versions)
}

/// Publishes every artifact and workflow in `out_dir`. Returns false if anything failed.
async fn publish(out_dir: &Path) -> bool {
    let master_url = match get_master_url() {
        Ok(url) => url,
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    };

    let client = client();
    let mut ok = true;

    let tasks_dir = out_dir.join("tasks");
    if tasks_dir.exists() {
        for entry in fs::read_dir(&tasks_dir)
            .unwrap_or_else(|_| fs::read_dir(".").unwrap())
            .flatten()
        {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("zst") {
                continue;
            }
            let bytes = fs::read(&path).unwrap_or_default();
            let part = multipart::Part::bytes(bytes)
                .file_name(path.file_name().unwrap().to_string_lossy().to_string())
                .mime_str("application/octet-stream")
                .unwrap();
            let form = multipart::Form::new().part("artifact", part);
            let url = format!("{}/tasks", master_url.trim_end_matches('/'));
            let resp = client.post(url).multipart(form).send().await;
            match resp {
                Ok(resp) if resp.status().is_success() => {
                    println!("Published task: {}", path.display());
                }
                Ok(resp) => {
                    ok = false;
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    eprintln!("Failed to publish task {}: {status} {body}", path.display());
                }
                Err(err) => {
                    ok = false;
                    eprintln!("Failed to publish task {}: {}", path.display(), err);
                }
            }
        }
    }

    let workflows_dir = out_dir.join("workflows");
    if workflows_dir.exists() {
        for entry in fs::read_dir(&workflows_dir)
            .unwrap_or_else(|_| fs::read_dir(".").unwrap())
            .flatten()
        {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let raw = fs::read_to_string(&path).unwrap_or_default();
            let req: WorkflowUploadRequest = match serde_json::from_str(&raw) {
                Ok(req) => req,
                Err(err) => {
                    ok = false;
                    eprintln!("Invalid workflow file {}: {}", path.display(), err);
                    continue;
                }
            };
            let url = format!("{}/workflows", master_url.trim_end_matches('/'));
            let resp = client.post(url).json(&req).send().await;
            match resp {
                Ok(resp) if resp.status().is_success() => {
                    println!("Published workflow: {}", path.display());
                    let published: Published = resp.json().await.unwrap_or_default();
                    for task in published.unchecked {
                        eprintln!(
                            "warning: calls of {task} are not checked: it was built without a \
                             signature"
                        );
                    }
                }
                Ok(resp) => {
                    ok = false;
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    eprintln!(
                        "Failed to publish workflow {}: {status} {body}",
                        path.display()
                    );
                }
                Err(err) => {
                    ok = false;
                    eprintln!("Failed to publish workflow {}: {}", path.display(), err);
                }
            }
        }
    }
    ok
}

/// `name=value`, the value as JSON, or else as a string.
fn parse_input(arg: &str) -> Result<(String, serde_json::Value), String> {
    let (name, value) = arg
        .split_once('=')
        .ok_or_else(|| format!("expected NAME=VALUE, got {arg:?}"))?;
    let value = serde_json::from_str(value)
        .unwrap_or_else(|_| serde_json::Value::String(value.to_string()));
    Ok((name.to_string(), value))
}

async fn run_workflow(
    workflow_id: &str,
    version: &str,
    inputs: serde_json::Map<String, serde_json::Value>,
    wait: bool,
) {
    let master_url = match get_master_url() {
        Ok(url) => url,
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    };
    let master_url = master_url.trim_end_matches('/').to_string();

    let client = client();
    let req = RunCreateRequest {
        workflow_id: workflow_id.to_string(),
        version: version.to_string(),
        inputs,
    };
    let resp = match client
        .post(format!("{master_url}/runs"))
        .json(&req)
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => resp,
        Ok(resp) => {
            let status = resp.status();
            eprintln!(
                "Failed to create run: {status} {}",
                resp.text().await.unwrap_or_default()
            );
            std::process::exit(1);
        }
        Err(err) => {
            eprintln!("Failed to create run: {}", err);
            std::process::exit(1);
        }
    };
    let created: RunCreateResponse = match resp.json().await {
        Ok(created) => created,
        Err(err) => {
            eprintln!("Invalid response: {err}");
            std::process::exit(1);
        }
    };
    println!("Run created: {}", created.run_id);
    if !wait {
        return;
    }

    let run = created.run_id.to_string();
    let url = format!("{master_url}/runs/{run}");
    // The run goes on while the orchestrator restarts: keep asking, for a while.
    let patience = std::time::Duration::from_secs(120);
    let mut failing_since: Option<std::time::Instant> = None;
    loop {
        let fetched = match client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => resp.json::<JsonValue>().await.ok(),
            Ok(resp) if resp.status().is_client_error() => {
                eprintln!("Failed to fetch status: {}", resp.status());
                std::process::exit(1);
            }
            _ => None,
        };
        let Some(mut status) = fetched else {
            let since = *failing_since.get_or_insert_with(std::time::Instant::now);
            if since.elapsed() > patience {
                eprintln!("The orchestrator has not answered for {patience:?}; giving up");
                std::process::exit(1);
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            continue;
        };
        failing_since = None;
        if status["status"] != "running" {
            // The status counts them; print them.
            for what in ["results", "failures"] {
                match all_pages(&client, &master_url, &run, what).await {
                    Ok(items) => status[what] = JsonValue::Array(items),
                    Err(err) => {
                        eprintln!("Failed to fetch {what}: {err:#}");
                        std::process::exit(1);
                    }
                }
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&status).unwrap_or_default()
            );
            if status["status"] != "succeeded" {
                std::process::exit(2);
            }
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

/// Why a request to the orchestrator failed.
enum Failed {
    /// It answered no: waiting changes nothing (a wrong run id, a token it does not take).
    Refused(anyhow::Error),
    /// It did not answer, or failed itself: it may be restarting.
    Unavailable(anyhow::Error),
}

impl From<Failed> for anyhow::Error {
    fn from(failed: Failed) -> Self {
        match failed {
            Failed::Refused(err) | Failed::Unavailable(err) => err,
        }
    }
}

/// One page of a run's `what` (`results` or `failures`) from the `after`th on.
async fn fetch_page(
    client: &reqwest::Client,
    master_url: &str,
    run: &str,
    what: &str,
    query: PageQuery,
) -> Result<Page<JsonValue>, Failed> {
    let mut url = format!("{master_url}/runs/{run}/{what}?after={}", query.after);
    if let Some(wait_ms) = query.wait_ms {
        url.push_str(&format!("&wait_ms={wait_ms}"));
    }
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|err| Failed::Unavailable(err.into()))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        let err = anyhow::anyhow!("{status} {body}");
        return Err(match status.is_client_error() {
            true => Failed::Refused(err),
            false => Failed::Unavailable(err),
        });
    }
    response
        .json()
        .await
        .map_err(|err| Failed::Unavailable(err.into()))
}

/// [`fetch_page`], asked again while the orchestrator does not answer, for up to two minutes:
/// a run goes on while it restarts.
async fn fetch_page_patiently(
    client: &reqwest::Client,
    master_url: &str,
    run: &str,
    what: &str,
    query: PageQuery,
) -> anyhow::Result<Page<JsonValue>> {
    let patience = std::time::Duration::from_secs(120);
    let started = std::time::Instant::now();
    loop {
        match fetch_page(client, master_url, run, what, query).await {
            Ok(page) => return Ok(page),
            Err(Failed::Unavailable(err)) if started.elapsed() < patience => {
                say_waiting(&err);
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
            Err(failed) => return Err(failed.into()),
        }
    }
}

/// Says, once a second at most, that the orchestrator is not answering.
fn say_waiting(err: &anyhow::Error) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static LAST: AtomicU64 = AtomicU64::new(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    if LAST.swap(now, Ordering::Relaxed) != now {
        eprintln!("waiting for the orchestrator: {err:#}");
    }
}

/// Every one of a run's `what` so far.
async fn all_pages(
    client: &reqwest::Client,
    master_url: &str,
    run: &str,
    what: &str,
) -> anyhow::Result<Vec<JsonValue>> {
    let mut items = Vec::new();
    loop {
        let query = PageQuery {
            after: items.len() as u64,
            ..PageQuery::default()
        };
        let page = fetch_page_patiently(client, master_url, run, what, query).await?;
        if page.items.is_empty() {
            return Ok(items);
        }
        items.extend(page.items);
    }
}

/// How long a followed page waits on the orchestrator for a report.
const FOLLOW_WAIT_MS: u64 = 20_000;

/// Prints a run's `what`, a JSON value per line; with `follow`, as they arrive, until the run
/// ends.
async fn print_results(run: &str, what: &str, follow: bool) -> anyhow::Result<()> {
    let master_url = get_master_url().map_err(anyhow::Error::msg)?;
    let master_url = master_url.trim_end_matches('/');
    let client = client();
    // Following, an empty page waits on the orchestrator for the next report, or the run's end.
    let mut query = PageQuery {
        wait_ms: follow.then_some(FOLLOW_WAIT_MS),
        ..PageQuery::default()
    };
    loop {
        let page = fetch_page_patiently(&client, master_url, run, what, query).await?;
        for item in &page.items {
            println!("{item}");
        }
        query.after = page.next;
        // A page read once the run ended holds everything up to its end.
        let done = page.status != RunStatus::Running;
        if page.items.is_empty() && (!follow || done) {
            return Ok(());
        }
    }
}

async fn run_status(run_id: &str) {
    let master_url = match get_master_url() {
        Ok(url) => url,
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    };
    let client = client();
    let url = format!("{}/runs/{}", master_url.trim_end_matches('/'), run_id);
    match client.get(url).send().await {
        Ok(resp) if resp.status().is_success() => {
            let json: JsonValue = resp.json().await.unwrap_or_default();
            println!(
                "{}",
                serde_json::to_string_pretty(&json).unwrap_or_default()
            );
        }
        Ok(resp) => {
            eprintln!("Failed to fetch status: {}", resp.status());
        }
        Err(err) => {
            eprintln!("Failed to fetch status: {}", err);
        }
    }
}

async fn list_workers() {
    let master_url = match get_master_url() {
        Ok(url) => url,
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    };
    let client = client();
    let url = format!("{}/workers", master_url.trim_end_matches('/'));
    match client.get(url).send().await {
        Ok(resp) if resp.status().is_success() => {
            let json: JsonValue = resp.json().await.unwrap_or_default();
            println!(
                "{}",
                serde_json::to_string_pretty(&json).unwrap_or_default()
            );
        }
        Ok(resp) => {
            eprintln!("Failed to fetch workers: {}", resp.status());
        }
        Err(err) => {
            eprintln!("Failed to fetch workers: {}", err);
        }
    }
}

async fn login() {
    let config_path = get_config_path();

    print!("Enter orchestrator URL: ");
    io::stdout().flush().unwrap();
    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();
    let master_url = input.trim().to_string();

    if master_url.is_empty() {
        eprintln!("Error: Orchestrator URL cannot be empty");
        std::process::exit(1);
    }

    println!("Performing health check on: {}", master_url);

    match perform_health_check(&master_url).await {
        Ok(_) => {
            println!("✓ Orchestrator is healthy");
            save_master_url(&config_path, &master_url).unwrap_or_else(|e| {
                eprintln!("Warning: Could not save URL: {}", e);
            });
        }
        Err(e) => {
            eprintln!("✗ Health check failed: {}", e);
            std::process::exit(1);
        }
    }
}

fn get_config_path() -> PathBuf {
    let mut config_dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    config_dir.push(".namu");
    fs::create_dir_all(&config_dir).ok();
    config_dir.push("config.txt");
    config_dir
}

fn get_saved_master_url(config_path: &Path) -> Option<String> {
    fs::read_to_string(config_path).ok().and_then(|content| {
        content
            .lines()
            .find(|line| line.starts_with("orchestrator_url="))
            .map(|line| line.strip_prefix("orchestrator_url=").unwrap().to_string())
    })
}

fn save_master_url(config_path: &Path, url: &str) -> io::Result<()> {
    let mut config_content = fs::read_to_string(config_path).unwrap_or_default();

    config_content = config_content
        .lines()
        .filter(|line| !line.starts_with("orchestrator_url="))
        .collect::<Vec<_>>()
        .join("\n");

    if !config_content.is_empty() {
        config_content.push('\n');
    }
    config_content.push_str(&format!("orchestrator_url={}", url));

    fs::write(config_path, config_content)
}

async fn perform_health_check(url: &str) -> Result<(), String> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("Invalid URL format - must start with http:// or https://".to_string());
    }

    let client = client();
    let health_url = format!("{}/healthz", url.trim_end_matches('/'));

    match client.get(&health_url).send().await {
        Ok(response) => {
            if response.status().is_success() {
                Ok(())
            } else {
                Err(format!(
                    "Health check failed with status: {}",
                    response.status()
                ))
            }
        }
        Err(e) => Err(format!("Failed to connect to server: {}", e)),
    }
}

pub fn get_master_url() -> Result<String, String> {
    if let Ok(url) = std::env::var("NAMU_ORCH_URL")
        && !url.trim().is_empty()
    {
        return Ok(url);
    }
    let config_path = get_config_path();
    get_saved_master_url(&config_path)
        .ok_or_else(|| "No orchestrator URL configured. Run 'namu login' first.".to_string())
}

/// The client for the orchestrator's API: with the token in `NAMU_TOKEN`, trusting the CA in
/// `NAMU_TLS_CA` too, if set (a cluster's own CA; otherwise the system's roots).
fn client() -> reqwest::Client {
    let build = || -> anyhow::Result<reqwest::Client> {
        let mut builder = reqwest::Client::builder();
        if let Ok(token) = std::env::var("NAMU_TOKEN") {
            let mut auth = reqwest::header::HeaderValue::from_str(&format!("Bearer {token}"))
                .map_err(|_| anyhow::anyhow!("NAMU_TOKEN is not a valid header value"))?;
            auth.set_sensitive(true);
            let mut headers = reqwest::header::HeaderMap::new();
            headers.insert(reqwest::header::AUTHORIZATION, auth);
            builder = builder.default_headers(headers);
        }
        if let Ok(ca) = std::env::var("NAMU_TLS_CA") {
            let pem =
                fs::read(&ca).map_err(|err| anyhow::anyhow!("reading NAMU_TLS_CA {ca}: {err}"))?;
            builder = builder.add_root_certificate(reqwest::Certificate::from_pem(&pem)?);
        }
        Ok(builder.build()?)
    };
    build().unwrap_or_else(|err| {
        eprintln!("Error: {err:#}");
        std::process::exit(1);
    })
}

/// A new token for `role`: printed, and its hash added to `tokens` (or printed, if not given).
fn make_token(role: &str, tokens: Option<&Path>) -> anyhow::Result<()> {
    anyhow::ensure!(
        matches!(role, "client" | "worker"),
        "the role is `client` or `worker`, not {role:?}"
    );
    let mut secret = [0u8; 32];
    getrandom::fill(&mut secret).map_err(|err| anyhow::anyhow!("no randomness: {err}"))?;
    let token: String = std::iter::once("namu_".to_string())
        .chain(secret.iter().map(|b| format!("{b:02x}")))
        .collect();
    let line = format!(
        "{role} sha256:{:x}\n",
        sha2::Sha256::digest(token.as_bytes())
    );
    match tokens {
        Some(path) => {
            use std::io::Write as _;
            let mut file = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)?;
            file.write_all(line.as_bytes())?;
        }
        None => eprint!("{line}"),
    }
    println!("{token}");
    Ok(())
}

/// A cluster CA, and one member certificate for every member, in `out`.
fn make_certs(out: &Path, hosts: &[String]) -> anyhow::Result<()> {
    use namu_engine::transport::certs::ClusterCa;
    fs::create_dir_all(out)?;
    let ca = ClusterCa::new()?;
    let member = ca.member(hosts)?;
    fs::write(out.join("ca.pem"), ca.pem())?;
    fs::write(out.join("member.pem"), member.cert)?;
    let key = out.join("member.key");
    fs::write(&key, member.key)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&key, fs::Permissions::from_mode(0o600))?;
    }
    println!(
        "wrote ca.pem, member.pem and member.key to {}",
        out.display()
    );
    Ok(())
}

fn show_version() {
    const VERSION: &str = env!("CARGO_PKG_VERSION");
    println!("namu v{}", VERSION);
}
