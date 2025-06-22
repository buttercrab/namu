use crate::{DynamicTaskContext, Task, Value};

/// Function type that packs multiple Value inputs into a single Value.
///
/// The engine relies on this when a task takes >1 arguments.
pub type PackFn = fn(Vec<Value>) -> Value;
/// Function type that unpacks a Value produced by a task into multiple
/// `Value`s corresponding to the task outputs.
pub type UnpackFn = fn(Value) -> Vec<Value>;

/// Static description of a task.  Instances of this struct are gathered at
/// compile time via the `inventory` crate and consumed by the engine at
/// runtime to register all available tasks automatically.
#[derive(Clone, Copy)]
pub struct TaskEntry {
    /// Human-readable task name.  Must match the identifier used inside the
    /// workflow JSON so the engine can map calls to implementations.
    pub name: &'static str,
    /// Optional metadata – currently just the author for demonstration.
    pub author: &'static str,
    /// Factory that returns a boxed task instance.
    pub create: fn() -> Box<dyn Task<usize, DynamicTaskContext<usize>> + Send + Sync>,
    /// SemVer-style version string of the task implementation.
    pub version: &'static str,
    /// Optional pack helper.  `None` means the task takes exactly one input
    /// and no special packing is required.
    pub pack: Option<PackFn>,
    /// Optional unpack helper.  `None` means the task returns exactly one
    /// value and no special unpacking is required.
    pub unpack: Option<UnpackFn>,
}

// Gather all submitted `TaskEntry`s across the dependency graph.
inventory::collect!(TaskEntry);
