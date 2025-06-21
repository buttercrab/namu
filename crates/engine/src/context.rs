use std::hash::Hash;
use std::{cmp::Ordering, fmt};

use namu_core::{Value, ValueId};

pub mod dynamic_context;
pub mod naive_context;
pub mod static_context;

pub trait ContextManager: Send + Sync {
    type ContextId: Clone + Eq + Hash + Send + Sync + fmt::Debug + 'static;

    fn create_context(&self) -> Self::ContextId;

    fn compare_context(&self, a: Self::ContextId, b: Self::ContextId) -> Ordering;

    fn add_value(
        &self,
        context_id: Self::ContextId,
        val_id: ValueId,
        value: Value,
    ) -> Self::ContextId;

    fn get_value(&self, context_id: Self::ContextId, val_id: ValueId) -> Value;

    fn get_values(&self, context_id: Self::ContextId, val_ids: &[ValueId]) -> Vec<Value>;

    fn remove_context(&self, context_id: Self::ContextId);
}
