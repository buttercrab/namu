use crate::{DynamicTaskContext, Task, Value};

pub type PackFn = fn(Vec<Value>) -> Value;
pub type UnpackFn = fn(Value) -> Vec<Value>;
pub type TaskImpl = Box<dyn Task<DynamicTaskContext> + Send + Sync>;

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
