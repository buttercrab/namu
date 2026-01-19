use std::sync::OnceLock;

use hashbrown::HashMap;

use crate::{DynamicTaskContext, Task, Value};

pub type PackFn = fn(Vec<Value>) -> Value;
pub type UnpackFn = fn(Value) -> Vec<Value>;
pub type TaskImpl = Box<dyn Task<DynamicTaskContext> + Send + Sync>;
pub type DeserializeFn =
    fn(&mut dyn erased_serde::Deserializer) -> Result<Value, erased_serde::Error>;

#[derive(Clone, Copy)]
pub struct TaskEntry {
    pub name: &'static str,
    pub author: &'static str,
    pub create: fn() -> TaskImpl,
    pub version: &'static str,
    pub pack: Option<PackFn>,
    pub unpack: Option<UnpackFn>,
}

inventory::collect!(TaskEntry);

pub fn get_tasks() -> HashMap<String, TaskEntry> {
    static TASKS: OnceLock<HashMap<String, TaskEntry>> = OnceLock::new();
    TASKS
        .get_or_init(|| {
            inventory::iter::<TaskEntry>
                .into_iter()
                .map(|e| (e.name.to_string(), *e))
                .collect()
        })
        .clone()
}

#[derive(Clone, Copy)]
pub struct TypeEntry {
    pub name: &'static str,
    pub type_id: &'static str,
    pub deserialize: DeserializeFn,
}

inventory::collect!(TypeEntry);

pub fn get_types() -> HashMap<String, TypeEntry> {
    static TYPES: OnceLock<HashMap<String, TypeEntry>> = OnceLock::new();
    TYPES
        .get_or_init(|| {
            inventory::iter::<TypeEntry>
                .into_iter()
                .map(|e| (e.name.to_string(), *e))
                .collect()
        })
        .clone()
}

#[derive(Clone, Copy)]
pub struct WorkflowEntry {
    pub id: &'static str,
    pub build: fn() -> crate::ir::Workflow,
}

inventory::collect!(WorkflowEntry);

pub fn get_workflows() -> HashMap<String, WorkflowEntry> {
    static WORKFLOWS: OnceLock<HashMap<String, WorkflowEntry>> = OnceLock::new();
    WORKFLOWS
        .get_or_init(|| {
            inventory::iter::<WorkflowEntry>
                .into_iter()
                .map(|e| (e.id.to_string(), *e))
                .collect()
        })
        .clone()
}
