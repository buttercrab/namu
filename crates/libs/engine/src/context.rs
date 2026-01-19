use std::cmp::Ordering;

use namu_core::{ContextId, Value, ValueId};

pub mod dynamic_context;
pub mod naive_context;
pub mod static_context;

pub trait ContextManager: Send + Sync {
    fn create_context(&self) -> ContextId;

    fn compare_context(&self, a: ContextId, b: ContextId) -> Ordering;

    fn add_value(&self, context_id: ContextId, val_id: ValueId, value: Value) -> ContextId;

    fn get_value(&self, context_id: ContextId, val_id: ValueId) -> Value;

    fn get_values(&self, context_id: ContextId, val_ids: &[ValueId]) -> Vec<Value>;

    fn remove_context(&self, context_id: ContextId);
}
