use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, anyhow};
use toml_edit::{DocumentMut, InlineTable, Item, Value};

use crate::config::{NamuConfig, resolve_workflows_manifest, workflow_package_name};

pub fn export_workflows(cfg: &NamuConfig) -> anyhow::Result<PathBuf> {
    let manifest_path = resolve_workflows_manifest(cfg)?;
    let workflow_name = workflow_package_name(&manifest_path)?;

    let export_root = cfg.root.join("target/namu/exporter");
    let export_src = export_root.join("src");
    let ir_dir = cfg.root.join("target/namu/ir");

    fs::create_dir_all(&export_src).context("create exporter src dir")?;
    fs::create_dir_all(&ir_dir).context("create workflow ir dir")?;

    write_exporter_manifest(&export_root, &manifest_path, &workflow_name)?;
    write_exporter_main(&export_src, &workflow_name, &ir_dir)?;

    let status = Command::new("cargo")
        .arg("run")
        .arg("--manifest-path")
        .arg(export_root.join("Cargo.toml"))
        .arg("--release")
        .env(
            "CARGO_TARGET_DIR",
            cfg.root.join("target/namu/exporter-target"),
        )
        .status()
        .context("run workflow exporter")?;
    if !status.success() {
        return Err(anyhow!("workflow export failed"));
    }

    Ok(ir_dir)
}

fn write_exporter_manifest(
    export_root: &Path,
    workflow_manifest: &Path,
    workflow_name: &str,
) -> anyhow::Result<()> {
    let namu_dep = read_namu_dependency(workflow_manifest)?;
    let mut doc = DocumentMut::new();

    doc["package"]["name"] = Value::from("namu-exporter").into();
    doc["package"]["version"] = Value::from("0.1.0").into();
    doc["package"]["edition"] = Value::from("2021").into();

    let deps = doc["dependencies"].or_insert(Item::Table(toml_edit::Table::new()));
    let deps = deps.as_table_mut().expect("dependencies table");

    deps.insert("namu", namu_dep);

    let workflow_path = workflow_manifest
        .parent()
        .unwrap()
        .canonicalize()
        .context("canonicalize workflow crate path")?;
    let mut workflow_dep = InlineTable::new();
    workflow_dep.insert(
        "path",
        Value::from(workflow_path.to_string_lossy().to_string()),
    );
    deps.insert(workflow_name, Item::Value(Value::InlineTable(workflow_dep)));

    fs::write(export_root.join("Cargo.toml"), doc.to_string())
        .context("write exporter Cargo.toml")?;
    Ok(())
}

fn write_exporter_main(
    export_src: &Path,
    workflow_name: &str,
    ir_dir: &Path,
) -> anyhow::Result<()> {
    let main_rs = format!(
        "#[allow(unused_imports)]\nuse {workflow_name} as _;\n\nfn main() {{\n    namu::export::write_all(\"{}\").expect(\"export workflows\");\n}}\n",
        ir_dir.to_string_lossy()
    );
    fs::write(export_src.join("main.rs"), main_rs).context("write exporter main.rs")?;
    Ok(())
}

fn read_namu_dependency(workflow_manifest: &Path) -> anyhow::Result<Item> {
    let raw = fs::read_to_string(workflow_manifest).context("read workflow Cargo.toml")?;
    let doc = raw.parse::<DocumentMut>()?;
    let dep = doc
        .get("dependencies")
        .and_then(|deps| deps.get("namu"))
        .ok_or_else(|| anyhow!("workflows crate must depend on namu"))?;

    if let Item::Value(Value::String(value)) = dep {
        let mut inline = InlineTable::new();
        inline.insert("version", Value::from(value.value().to_string()));
        return Ok(Item::Value(Value::InlineTable(inline)));
    }

    if let Item::Value(Value::InlineTable(table)) = dep {
        let mut inline = InlineTable::new();
        for key in [
            "version",
            "registry",
            "path",
            "git",
            "rev",
            "branch",
            "tag",
            "features",
            "default-features",
        ] {
            if let Some(item) = table.get(key) {
                let value = item.clone();
                if key == "path" {
                    if let Some(path_str) = value.as_str() {
                        let resolved = workflow_manifest
                            .parent()
                            .unwrap()
                            .join(path_str)
                            .canonicalize()
                            .context("canonicalize namu path dependency")?;
                        inline.insert(key, Value::from(resolved.to_string_lossy().to_string()));
                        continue;
                    }
                }
                inline.insert(key, value);
            }
        }
        return Ok(Item::Value(Value::InlineTable(inline)));
    }

    Err(anyhow!("unsupported namu dependency format"))
}
