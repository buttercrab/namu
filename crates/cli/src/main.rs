use clap::{Parser, Subcommand};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
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
    /// Compile tasks from the tasks directory
    Compile {
        /// Path to tasks directory (default: ./tasks)
        #[arg(short, long, default_value = "./tasks")]
        tasks_dir: PathBuf,
        /// Output directory for compiled binaries (default: ./outputs/tasks)
        #[arg(short, long, default_value = "./outputs/tasks")]
        output_dir: PathBuf,
        /// Target architectures to build for (default: all supported targets)
        #[arg(long, value_delimiter = ',')]
        targets: Option<Vec<String>>,
        /// Use subdirectories for each target instead of filename suffixes
        #[arg(long)]
        use_subdirs: bool,
    },
    /// Check types for pipeline definitions
    Check {
        /// Path to tasks directory (default: ./tasks)
        #[arg(short, long, default_value = "./tasks")]
        tasks_dir: PathBuf,
    },
    /// Show version information
    Version,
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Compile {
            tasks_dir,
            output_dir,
            targets,
            use_subdirs,
        } => {
            build_tasks(&tasks_dir, &output_dir, targets, use_subdirs);
        }
        Commands::Check { tasks_dir } => {
            check_tasks(&tasks_dir);
        }
        Commands::Version => {
            show_version();
        }
    }
}

fn build_tasks(
    tasks_dir: &Path,
    output_dir: &Path,
    targets: Option<Vec<String>>,
    use_subdirs: bool,
) {
    println!("Building tasks from: {}", tasks_dir.display());
    println!("Output directory: {}", output_dir.display());

    if !tasks_dir.exists() {
        eprintln!(
            "Error: Tasks directory '{}' does not exist",
            tasks_dir.display()
        );
        std::process::exit(1);
    }

    // Default target list if none provided
    let default_targets = vec![
        "aarch64-apple-darwin".to_string(),
        "aarch64-unknown-linux-gnu".to_string(),
        "x86_64-unknown-linux-gnu".to_string(),
    ];
    let build_targets = targets.as_ref().unwrap_or(&default_targets);

    println!("Building for targets: {}", build_targets.join(", "));

    // Create output directory if it doesn't exist
    if let Err(e) = fs::create_dir_all(output_dir) {
        eprintln!("Error creating output directory: {}", e);
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

        println!("Building task: {}", task_name);

        // Build for each target
        for target in build_targets {
            println!("  Building for target: {}", target);

            // Install target if not already installed
            let install_output = Command::new("rustup")
                .args(["target", "add", target])
                .output();

            if let Err(e) = install_output {
                eprintln!("  ✗ Error installing target {}: {}", target, e);
                continue;
            }

            let output = Command::new("cargo")
                .args(["build", "--release", "--target", target, "--manifest-path"])
                .arg(cargo_path)
                .output();

            match output {
                Ok(output) => {
                    if output.status.success() {
                        println!("  ✓ Successfully built task: {} for {}", task_name, target);

                        // Determine binary extension for Windows targets
                        let binary_name = if target.contains("windows") {
                            format!("{}.exe", task_name)
                        } else {
                            task_name.to_string()
                        };

                        // Copy binary to output directory
                        let binary_src = task_dir
                            .join("target")
                            .join(target)
                            .join("release")
                            .join(&binary_name);

                        let binary_dst = if use_subdirs {
                            let target_dir = output_dir.join(target);
                            if let Err(e) = fs::create_dir_all(&target_dir) {
                                eprintln!(
                                    "  ✗ Failed to create target directory {}: {}",
                                    target_dir.display(),
                                    e
                                );
                                continue;
                            }
                            target_dir.join(&binary_name)
                        } else {
                            let suffix = match target.as_str() {
                                "aarch64-apple-darwin" => "darwin-arm64",
                                "aarch64-unknown-linux-gnu" => "linux-arm64",
                                "x86_64-unknown-linux-gnu" => "linux-amd64",
                                _ => target,
                            };
                            let filename = if target.contains("windows") {
                                format!("{}-{}.exe", task_name, suffix)
                            } else {
                                format!("{}-{}", task_name, suffix)
                            };
                            output_dir.join(filename)
                        };

                        if binary_src.exists() {
                            if let Err(e) = fs::copy(&binary_src, &binary_dst) {
                                eprintln!(
                                    "  ✗ Failed to copy binary for task {} ({}): {}",
                                    task_name, target, e
                                );
                            } else {
                                println!("  ✓ Copied binary to: {}", binary_dst.display());
                            }
                        } else {
                            eprintln!("  ✗ Binary not found at: {}", binary_src.display());
                        }
                    } else {
                        eprintln!("  ✗ Failed to build task: {} for {}", task_name, target);
                        eprintln!("  Error: {}", String::from_utf8_lossy(&output.stderr));
                    }
                }
                Err(e) => {
                    eprintln!(
                        "  ✗ Error executing cargo for task {} ({}): {}",
                        task_name, target, e
                    );
                }
            }
        }
    }
}

fn check_tasks(tasks_dir: &Path) {
    println!("Checking tasks in: {}", tasks_dir.display());

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

fn show_version() {
    const VERSION: &str = env!("CARGO_PKG_VERSION");
    println!("namu v{}", VERSION);
}
