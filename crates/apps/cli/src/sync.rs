use std::fs;

use anyhow::{Context, anyhow};
use toml_edit::{DocumentMut, InlineTable, Item, Table, Value};

use crate::config::{NamuConfig, resolve_registry, resolve_workflows_manifest};

pub fn sync_config(cfg: &NamuConfig) -> anyhow::Result<()> {
    ensure_registry_config(cfg)?;
    update_workflow_cargo(cfg)?;
    Ok(())
}

fn ensure_registry_config(cfg: &NamuConfig) -> anyhow::Result<()> {
    if cfg.registries.is_empty() {
        return Ok(());
    }

    let cargo_dir = cfg.root.join(".cargo");
    fs::create_dir_all(&cargo_dir).context("create .cargo directory")?;
    let config_path = cargo_dir.join("config.toml");

    let mut doc = if config_path.exists() {
        fs::read_to_string(&config_path)
            .context("read .cargo/config.toml")?
            .parse::<DocumentMut>()?
    } else {
        DocumentMut::new()
    };

    let registries = doc.entry("registries").or_insert(Item::Table(Table::new()));
    let registries = registries
        .as_table_mut()
        .ok_or_else(|| anyhow!("registries must be a table"))?;

    for (name, registry) in &cfg.registries {
        let entry = registries.entry(name).or_insert(Item::Table(Table::new()));
        let entry = entry
            .as_table_mut()
            .ok_or_else(|| anyhow!("registries.{name} must be a table"))?;
        entry["index"] = Value::from(registry.index.clone()).into();
    }

    fs::write(&config_path, doc.to_string()).context("write .cargo/config.toml")?;
    Ok(())
}

fn update_workflow_cargo(cfg: &NamuConfig) -> anyhow::Result<()> {
    let manifest_path = resolve_workflows_manifest(cfg)?;
    let raw = fs::read_to_string(&manifest_path).context("read workflow Cargo.toml")?;
    let mut doc = raw.parse::<DocumentMut>()?;

    let deps = doc
        .entry("dependencies")
        .or_insert(Item::Table(Table::new()));
    let deps = deps
        .as_table_mut()
        .ok_or_else(|| anyhow!("dependencies must be a table"))?;

    for task in cfg.tasks.values() {
        let registry = resolve_registry(cfg, task);
        let existing = deps.get(&task.crate_name);
        let mut inline = InlineTable::new();
        inline.insert("version", Value::from(task.version.clone()));
        if let Some(registry) = registry {
            inline.insert("registry", Value::from(registry));
        }
        if let Some(existing) = existing {
            if let Item::Value(Value::InlineTable(table)) = existing {
                if let Some(features) = table.get("features") {
                    inline.insert("features", features.clone());
                }
                if let Some(default_features) = table.get("default-features") {
                    inline.insert("default-features", default_features.clone());
                }
            }
        }

        deps.insert(&task.crate_name, Item::Value(Value::InlineTable(inline)));
    }

    fs::write(&manifest_path, doc.to_string()).context("write workflow Cargo.toml")?;
    Ok(())
}
