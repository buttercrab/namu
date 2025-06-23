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

#[derive(Clone, Copy)]
pub struct TypeEntry {
    pub name: &'static str,
    pub type_id: &'static str,
    pub deserialize: DeserializeFn,
}

inventory::collect!(TypeEntry);
