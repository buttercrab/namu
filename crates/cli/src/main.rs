use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use clap::{Parser, Subcommand};
use namu_proto::{RunCreateRequest, WorkflowUploadRequest};
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
    },
    /// Check run status
    Status { run_id: String },
    /// Fetch run events
    Logs {
        run_id: String,
        #[arg(short, long, default_value = "100")]
        limit: usize,
    },
    /// List workers
    Workers,
    /// Login and healthcheck orchestrator
    Login,
    /// Show version information
    Version,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Build {
            tasks_dir,
            workflows_dir,
            out_dir,
        } => {
            if let Err(err) = build(&tasks_dir, &workflows_dir, &out_dir) {
                eprintln!("Build failed: {err}");
                std::process::exit(1);
            }
        }
        Commands::Publish { out_dir } => {
            publish(&out_dir).await;
        }
        Commands::Run {
            workflow_id,
            version,
        } => {
            run_workflow(&workflow_id, &version).await;
        }
        Commands::Status { run_id } => {
            run_status(&run_id).await;
        }
        Commands::Logs { run_id, limit } => {
            run_logs(&run_id, limit).await;
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
    }
}

fn build(tasks_dir: &Path, workflows_dir: &Path, out_dir: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(out_dir.join("tasks"))?;
    fs::create_dir_all(out_dir.join("workflows"))?;

    build_tasks(tasks_dir, &out_dir.join("tasks"))?;
    build_workflows(workflows_dir, &out_dir.join("workflows"))?;
    Ok(())
}

fn build_tasks(tasks_dir: &Path, out_dir: &Path) -> anyhow::Result<()> {
    for entry in WalkDir::new(tasks_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name() == "manifest.json")
    {
        let task_dir = entry.path().parent().unwrap();
        let manifest_path = entry.path();
        let manifest_raw = fs::read_to_string(manifest_path)?;
        let mut manifest: namu_proto::TaskManifest = serde_json::from_str(&manifest_raw)?;

        if task_dir.join("Cargo.toml").exists() {
            let status = Command::new("cargo")
                .args(["build", "--release", "--manifest-path"])
                .arg(task_dir.join("Cargo.toml"))
                .status()?;
            if !status.success() {
                return Err(anyhow::anyhow!(
                    "cargo build failed for {}",
                    task_dir.display()
                ));
            }
        }

        let lib_path = find_library(task_dir)?;
        let lib_bytes = fs::read(&lib_path)?;
        let checksum = sha2::Sha256::digest(&lib_bytes);
        manifest.checksum = format!("sha256:{:x}", checksum);

        let tar_path = out_dir.join(format!("{}-{}.tar.zst", manifest.task_id, manifest.version));
        package_task(&tar_path, &manifest, &lib_path)?;
        println!("Built {}", tar_path.display());
    }
    Ok(())
}

fn find_library(task_dir: &Path) -> anyhow::Result<PathBuf> {
    let target_dir = task_dir.join("target/release");
    let mut candidates = Vec::new();
    if target_dir.exists() {
        for entry in fs::read_dir(&target_dir)? {
            let entry = entry?;
            let path = entry.path();
            let name = path.file_name().unwrap().to_string_lossy();
            if name.ends_with(".so") || name.ends_with(".dylib") || name.ends_with(".dll") {
                candidates.push(path);
            }
        }
    }
    if candidates.len() == 1 {
        Ok(candidates.remove(0))
    } else {
        Err(anyhow::anyhow!(
            "expected 1 library, found {}",
            candidates.len()
        ))
    }
}

fn package_task(
    path: &Path,
    manifest: &namu_proto::TaskManifest,
    lib_path: &Path,
) -> anyhow::Result<()> {
    let mut encoder = zstd::Encoder::new(fs::File::create(path)?, 3)?;
    {
        let mut tar = tar::Builder::new(&mut encoder);

        let manifest_json = serde_json::to_vec_pretty(manifest)?;
        let mut header = tar::Header::new_gnu();
        header.set_size(manifest_json.len() as u64);
        header.set_cksum();
        tar.append_data(&mut header, "manifest.json", manifest_json.as_slice())?;

        let lib_bytes = fs::read(lib_path)?;
        let mut header = tar::Header::new_gnu();
        header.set_size(lib_bytes.len() as u64);
        header.set_cksum();
        let lib_name = lib_path.file_name().unwrap().to_string_lossy();
        tar.append_data(&mut header, lib_name.as_ref(), lib_bytes.as_slice())?;

        tar.finish()?;
    }
    encoder.finish()?;
    Ok(())
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

async fn publish(out_dir: &Path) {
    let master_url = match get_master_url() {
        Ok(url) => url,
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    };

    let client = reqwest::Client::new();

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
                    eprintln!(
                        "Failed to publish task {}: {}",
                        path.display(),
                        resp.status()
                    );
                }
                Err(err) => {
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
                    eprintln!("Invalid workflow file {}: {}", path.display(), err);
                    continue;
                }
            };
            let url = format!("{}/workflows", master_url.trim_end_matches('/'));
            let resp = client.post(url).json(&req).send().await;
            match resp {
                Ok(resp) if resp.status().is_success() => {
                    println!("Published workflow: {}", path.display());
                }
                Ok(resp) => {
                    eprintln!(
                        "Failed to publish workflow {}: {}",
                        path.display(),
                        resp.status()
                    );
                }
                Err(err) => {
                    eprintln!("Failed to publish workflow {}: {}", path.display(), err);
                }
            }
        }
    }
}

async fn run_workflow(workflow_id: &str, version: &str) {
    let master_url = match get_master_url() {
        Ok(url) => url,
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    };

    let client = reqwest::Client::new();
    let req = RunCreateRequest {
        workflow_id: workflow_id.to_string(),
        version: version.to_string(),
    };
    let url = format!("{}/runs", master_url.trim_end_matches('/'));
    match client.post(url).json(&req).send().await {
        Ok(resp) if resp.status().is_success() => {
            let json: JsonValue = resp.json().await.unwrap_or_default();
            println!("Run created: {}", json);
        }
        Ok(resp) => {
            eprintln!("Failed to create run: {}", resp.status());
        }
        Err(err) => {
            eprintln!("Failed to create run: {}", err);
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
    let client = reqwest::Client::new();
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

async fn run_logs(run_id: &str, limit: usize) {
    let master_url = match get_master_url() {
        Ok(url) => url,
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    };
    let client = reqwest::Client::new();
    let url = format!(
        "{}/runs/{}/events?limit={}",
        master_url.trim_end_matches('/'),
        run_id,
        limit
    );
    match client.get(url).send().await {
        Ok(resp) if resp.status().is_success() => {
            let json: JsonValue = resp.json().await.unwrap_or_default();
            println!(
                "{}",
                serde_json::to_string_pretty(&json).unwrap_or_default()
            );
        }
        Ok(resp) => {
            eprintln!("Failed to fetch logs: {}", resp.status());
        }
        Err(err) => {
            eprintln!("Failed to fetch logs: {}", err);
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
    let client = reqwest::Client::new();
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

    let client = reqwest::Client::new();
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

fn show_version() {
    const VERSION: &str = env!("CARGO_PKG_VERSION");
    println!("namu v{}", VERSION);
}
