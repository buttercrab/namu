use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, anyhow};
use cargo_metadata::MetadataCommand;
use namu_proto::{TaskKind, TaskRuntime, TaskTrust};
use serde_json::Value as JsonValue;

#[derive(Clone, Debug)]
pub struct NamuConfig {
    pub root: PathBuf,
    pub project: ProjectConfig,
    pub registries: HashMap<String, RegistryConfig>,
    pub tasks: HashMap<String, TaskConfig>,
    pub workflows: WorkflowsConfig,
    pub build: BuildConfig,
}

#[derive(Clone, Debug)]
pub struct ProjectConfig {
    #[allow(dead_code)]
    pub name: String,
    pub workflows_crate: String,
    pub registry: Option<String>,
}

#[derive(Clone, Debug)]
pub struct RegistryConfig {
    pub index: String,
}

#[derive(Clone, Debug)]
pub struct TaskConfig {
    pub id: String,
    pub crate_name: String,
    pub version: String,
    pub registry: Option<String>,
    pub path: Option<PathBuf>,
    pub kind: TaskKind,
    pub runtime: TaskRuntime,
    pub trust: TaskTrust,
    pub requires_gpu: bool,
    pub resource_class: String,
    pub capabilities: Vec<String>,
    pub input_arity: usize,
    pub output_arity: usize,
    pub input_schema: JsonValue,
    pub output_schema: JsonValue,
}

#[derive(Clone, Debug)]
pub struct WorkflowsConfig {
    #[allow(dead_code)]
    pub export: WorkflowExport,
    pub entries: HashMap<String, WorkflowEntryConfig>,
}

#[derive(Clone, Debug)]
pub struct WorkflowEntryConfig {
    pub version: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkflowExport {
    Auto,
}

#[derive(Clone, Debug)]
pub struct BuildConfig {
    pub out_dir: PathBuf,
}

pub fn find_config(path: Option<PathBuf>) -> Option<PathBuf> {
    if let Some(path) = path {
        return Some(path);
    }
    let candidate = PathBuf::from("namu.toml");
    if candidate.exists() {
        return Some(candidate);
    }
    None
}

pub fn load_config(path: &Path) -> anyhow::Result<NamuConfig> {
    let raw = fs::read_to_string(path).context("read namu.toml")?;
    let doc: toml::Value = toml::from_str(&raw).context("parse namu.toml")?;
    let root = path
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));

    let project = parse_project(&doc)?;
    let registries = parse_registries(&doc)?;
    let tasks = parse_tasks(&doc, &root)?;
    let workflows = parse_workflows(&doc)?;
    let build = parse_build(&doc, &root)?;

    Ok(NamuConfig {
        root,
        project,
        registries,
        tasks,
        workflows,
        build,
    })
}

pub fn resolve_workflows_manifest(cfg: &NamuConfig) -> anyhow::Result<PathBuf> {
    let raw = cfg.project.workflows_crate.as_str();
    if raw.ends_with("Cargo.toml") {
        let path = cfg.root.join(raw);
        return Ok(path);
    }

    let path_candidate = cfg.root.join(raw);
    let manifest_candidate = path_candidate.join("Cargo.toml");
    if manifest_candidate.exists() {
        return Ok(manifest_candidate);
    }

    let metadata = MetadataCommand::new()
        .current_dir(&cfg.root)
        .no_deps()
        .exec()
        .context("load cargo metadata")?;
    let pkg = metadata
        .packages
        .iter()
        .find(|p| p.name == raw)
        .ok_or_else(|| anyhow!("workflows_crate not found: {raw}"))?;
    Ok(PathBuf::from(pkg.manifest_path.as_str()))
}

pub fn workflow_package_name(manifest_path: &Path) -> anyhow::Result<String> {
    let raw = fs::read_to_string(manifest_path).context("read workflow Cargo.toml")?;
    let doc = raw.parse::<toml_edit::DocumentMut>()?;
    let name = doc
        .get("package")
        .and_then(|pkg| pkg.get("name"))
        .and_then(|val| val.as_str())
        .ok_or_else(|| anyhow!("missing package.name in workflow Cargo.toml"))?;
    Ok(name.to_string())
}

pub fn resolve_registry(cfg: &NamuConfig, task: &TaskConfig) -> Option<String> {
    task.registry
        .clone()
        .or_else(|| cfg.project.registry.clone())
}

fn parse_project(doc: &toml::Value) -> anyhow::Result<ProjectConfig> {
    let table = doc
        .get("project")
        .and_then(|v| v.as_table())
        .ok_or_else(|| anyhow!("missing [project] section"))?;
    let name = get_string(table, "name")?;
    let workflows_crate = get_string(table, "workflows_crate")?;
    let registry = table
        .get("registry")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    Ok(ProjectConfig {
        name,
        workflows_crate,
        registry,
    })
}

fn parse_registries(doc: &toml::Value) -> anyhow::Result<HashMap<String, RegistryConfig>> {
    let mut registries = HashMap::new();
    let table = match doc.get("registry").and_then(|v| v.as_table()) {
        Some(t) => t,
        None => return Ok(registries),
    };

    for (name, value) in table {
        let entry = value
            .as_table()
            .ok_or_else(|| anyhow!("registry.{name} must be a table"))?;
        let index = get_string(entry, "index")?;
        registries.insert(name.to_string(), RegistryConfig { index });
    }

    Ok(registries)
}

fn parse_tasks(doc: &toml::Value, root: &Path) -> anyhow::Result<HashMap<String, TaskConfig>> {
    let mut tasks = HashMap::new();
    let table = match doc.get("tasks").and_then(|v| v.as_table()) {
        Some(t) => t,
        None => return Ok(tasks),
    };

    for (id, value) in table {
        let entry = value
            .as_table()
            .ok_or_else(|| anyhow!("tasks.{id} must be a table"))?;
        let crate_name = get_string(entry, "crate")?;
        let version = get_string(entry, "version")?;
        let registry = entry
            .get("registry")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let path = entry
            .get("path")
            .and_then(|v| v.as_str())
            .map(|p| root.join(p));

        let kind = parse_task_kind(get_string(entry, "kind")?.as_str())?;
        let runtime = parse_task_runtime(get_string(entry, "runtime")?.as_str())?;
        let trust = parse_task_trust(get_string(entry, "trust")?.as_str())?;
        let requires_gpu = entry
            .get("requires_gpu")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let resource_class = entry
            .get("resource_class")
            .and_then(|v| v.as_str())
            .unwrap_or("cpu.small")
            .to_string();
        let capabilities = entry
            .get("capabilities")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|| vec!["cpu".to_string()]);
        let input_arity = entry
            .get("input_arity")
            .and_then(|v| v.as_integer())
            .unwrap_or(0) as usize;
        let output_arity = entry
            .get("output_arity")
            .and_then(|v| v.as_integer())
            .unwrap_or(0) as usize;
        let input_schema = entry
            .get("input_schema")
            .map(toml_value_to_json)
            .unwrap_or(JsonValue::Null);
        let output_schema = entry
            .get("output_schema")
            .map(toml_value_to_json)
            .unwrap_or(JsonValue::Null);

        tasks.insert(
            id.to_string(),
            TaskConfig {
                id: id.to_string(),
                crate_name,
                version,
                registry,
                path,
                kind,
                runtime,
                trust,
                requires_gpu,
                resource_class,
                capabilities,
                input_arity,
                output_arity,
                input_schema,
                output_schema,
            },
        );
    }

    Ok(tasks)
}

fn parse_workflows(doc: &toml::Value) -> anyhow::Result<WorkflowsConfig> {
    let table = match doc.get("workflows").and_then(|v| v.as_table()) {
        Some(t) => t,
        None => {
            return Ok(WorkflowsConfig {
                export: WorkflowExport::Auto,
                entries: HashMap::new(),
            });
        }
    };

    let export = table
        .get("export")
        .and_then(|v| v.as_str())
        .map(parse_workflow_export)
        .transpose()?
        .unwrap_or(WorkflowExport::Auto);

    let mut entries = HashMap::new();
    for (name, value) in table {
        if name == "export" {
            continue;
        }
        let entry = value
            .as_table()
            .ok_or_else(|| anyhow!("workflows.{name} must be a table"))?;
        let version = get_string(entry, "version")?;
        entries.insert(name.to_string(), WorkflowEntryConfig { version });
    }

    Ok(WorkflowsConfig { export, entries })
}

fn parse_build(doc: &toml::Value, root: &Path) -> anyhow::Result<BuildConfig> {
    let out_dir = doc
        .get("build")
        .and_then(|v| v.as_table())
        .and_then(|t| t.get("out_dir"))
        .and_then(|v| v.as_str())
        .map(|s| root.join(s))
        .unwrap_or_else(|| root.join("dist"));

    Ok(BuildConfig { out_dir })
}

fn parse_task_kind(raw: &str) -> anyhow::Result<TaskKind> {
    match raw {
        "single" => Ok(TaskKind::Single),
        "batch" => Ok(TaskKind::Batch),
        "stream" => Ok(TaskKind::Stream),
        _ => Err(anyhow!("invalid task kind: {raw}")),
    }
}

fn parse_task_runtime(raw: &str) -> anyhow::Result<TaskRuntime> {
    match raw {
        "native" => Ok(TaskRuntime::Native),
        "wasm" => Ok(TaskRuntime::Wasm),
        _ => Err(anyhow!("invalid task runtime: {raw}")),
    }
}

fn parse_task_trust(raw: &str) -> anyhow::Result<TaskTrust> {
    match raw {
        "trusted" => Ok(TaskTrust::Trusted),
        "restricted" => Ok(TaskTrust::Restricted),
        "untrusted" => Ok(TaskTrust::Untrusted),
        _ => Err(anyhow!("invalid task trust: {raw}")),
    }
}

fn parse_workflow_export(raw: &str) -> anyhow::Result<WorkflowExport> {
    match raw {
        "auto" => Ok(WorkflowExport::Auto),
        _ => Err(anyhow!("unsupported workflows.export: {raw}")),
    }
}

fn get_string(table: &toml::value::Table, key: &str) -> anyhow::Result<String> {
    table
        .get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow!("missing {key}"))
}

fn toml_value_to_json(value: &toml::Value) -> JsonValue {
    serde_json::to_value(value).unwrap_or(JsonValue::Null)
}
