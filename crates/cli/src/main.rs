use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use clap::{Parser, Subcommand};
use reqwest::multipart;
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
    /// Compile tasks in the tasks directory
    Compile {
        /// Path to tasks directory (default: ./tasks)
        #[arg(short, long, default_value = "./tasks")]
        tasks_dir: PathBuf,
    },
    /// Login and healthcheck master server
    Login,
    /// Publish tasks to master server
    Publish {
        /// Path to tasks directory (default: ./tasks)
        #[arg(short, long, default_value = "./tasks")]
        tasks_dir: PathBuf,
    },
    /// Show version information
    Version,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Compile { tasks_dir } => {
            build_tasks(&tasks_dir, &PathBuf::new(), None, false);
        }
        Commands::Login => {
            login().await;
        }
        Commands::Publish { tasks_dir } => {
            publish_tasks(&tasks_dir).await;
        }
        Commands::Version => {
            show_version();
        }
    }
}

fn build_tasks(
    tasks_dir: &Path,
    _output_dir: &Path,
    _targets: Option<Vec<String>>,
    _use_subdirs: bool,
) {
    println!("Checking tasks from: {}", tasks_dir.display());

    if !tasks_dir.exists() {
        eprintln!(
            "Error: Tasks directory '{}' does not exist",
            tasks_dir.display()
        );
        std::process::exit(1);
    }

    // Find all Cargo.toml files in tasks directory
    for entry in WalkDir::new(tasks_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name() == "Cargo.toml")
    {
        let cargo_path = entry.path();
        let task_dir = cargo_path.parent().unwrap();
        let task_name = task_dir.file_name().unwrap().to_str().unwrap();

        println!("Checking task: {}", task_name);

        let output = Command::new("cargo")
            .args(["check", "--manifest-path"])
            .arg(cargo_path)
            .output();

        match output {
            Ok(output) => {
                if output.status.success() {
                    println!("✓ Task {} passed type check", task_name);
                } else {
                    eprintln!("✗ Task {} failed type check", task_name);
                    eprintln!("Error: {}", String::from_utf8_lossy(&output.stderr));
                }
            }
            Err(e) => {
                eprintln!(
                    "✗ Error executing cargo check for task {}: {}",
                    task_name, e
                );
            }
        }
    }
}

async fn login() {
    let config_path = get_config_path();

    // Always prompt for master server URL in login command
    print!("Enter master server URL: ");
    io::stdout().flush().unwrap();
    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();
    let master_url = input.trim().to_string();

    if master_url.is_empty() {
        eprintln!("Error: Master server URL cannot be empty");
        std::process::exit(1);
    }

    // Perform health check first
    println!("Performing health check on: {}", master_url);

    match perform_health_check(&master_url).await {
        Ok(_) => {
            println!("✓ Master server is healthy");

            // Save the new master server URL only after successful health check
            save_master_url(&config_path, &master_url).unwrap_or_else(|e| {
                eprintln!("Warning: Could not save master URL: {}", e);
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
            .find(|line| line.starts_with("master_url="))
            .map(|line| line.strip_prefix("master_url=").unwrap().to_string())
    })
}

fn save_master_url(config_path: &Path, url: &str) -> io::Result<()> {
    let mut config_content = fs::read_to_string(config_path).unwrap_or_default();

    // Remove existing master_url line if present
    config_content = config_content
        .lines()
        .filter(|line| !line.starts_with("master_url="))
        .collect::<Vec<_>>()
        .join("\n");

    // Add new master_url line
    if !config_content.is_empty() {
        config_content.push('\n');
    }
    config_content.push_str(&format!("master_url={}", url));

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

async fn publish_tasks(tasks_dir: &Path) {
    println!("Publishing tasks from: {}", tasks_dir.display());

    if !tasks_dir.exists() {
        eprintln!(
            "Error: Tasks directory '{}' does not exist",
            tasks_dir.display()
        );
        std::process::exit(1);
    }

    // Get master server URL
    let master_url = match get_master_url() {
        Ok(url) => url,
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    };

    let client = reqwest::Client::new();
    let mut form = multipart::Form::new();

    // Collect only source files (src/, Cargo.toml, Cargo.lock)
    for entry in WalkDir::new(tasks_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| {
            let path = e.path();
            let relative_path = path.strip_prefix(tasks_dir).unwrap();
            let path_str = relative_path.to_string_lossy();

            // Only include src/, Cargo.toml, and Cargo.lock files
            path_str.starts_with("src/")
                || path_str.ends_with("Cargo.toml")
                || path_str.ends_with("Cargo.lock")
                || path_str.contains("/src/")
                || path_str.contains("/Cargo.toml")
                || path_str.contains("/Cargo.lock")
        })
    {
        let file_path = entry.path();
        let relative_path = file_path.strip_prefix(tasks_dir).unwrap();

        match fs::read(file_path) {
            Ok(file_contents) => {
                let file_name = relative_path.to_string_lossy().to_string();
                form = form.part(
                    "files",
                    multipart::Part::bytes(file_contents)
                        .file_name(file_name.clone())
                        .mime_str("application/octet-stream")
                        .unwrap(),
                );
                println!("Added file: {}", file_name);
            }
            Err(e) => {
                eprintln!(
                    "Warning: Could not read file {}: {}",
                    file_path.display(),
                    e
                );
            }
        }
    }

    // Ensure URL doesn't end with slash and format properly
    let master_url = master_url.trim_end_matches('/');
    let upload_url = format!("{}/tasks/upload", master_url);
    println!("Uploading to: {}", upload_url);

    match client.post(&upload_url).multipart(form).send().await {
        Ok(response) => {
            if response.status().is_success() {
                match response.json::<serde_json::Value>().await {
                    Ok(json) => {
                        println!("✓ Tasks published successfully");
                        if let Some(files) = json.get("files") {
                            println!("Published files: {}", files);
                        }
                    }
                    Err(_) => println!("✓ Tasks published successfully"),
                }
            } else {
                eprintln!("✗ Failed to publish tasks: HTTP {}", response.status());
                if let Ok(text) = response.text().await {
                    eprintln!("Error details: {}", text);
                }
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("✗ Failed to publish tasks: {}", e);
            std::process::exit(1);
        }
    }
}

pub fn get_master_url() -> Result<String, String> {
    let config_path = get_config_path();
    get_saved_master_url(&config_path)
        .ok_or_else(|| "No master server URL configured. Run 'namu login' first.".to_string())
}

fn show_version() {
    const VERSION: &str = env!("CARGO_PKG_VERSION");
    println!("namu v{}", VERSION);
}
