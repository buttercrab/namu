use std::{any::Any, cmp::Ordering, hash::Hash, sync::Arc};

use namu_core::ir::ValueId;

pub mod dynamic_context;
pub mod naive_context;
pub mod static_context;

pub trait ContextManager {
    type ContextId: Clone + Eq + Hash + Send + Sync + 'static;

    fn create_context(&self) -> Self::ContextId;

    fn compare_context(&self, a: Self::ContextId, b: Self::ContextId) -> Ordering;

    fn add_value(
        &self,
        context_id: Self::ContextId,
        val_id: ValueId,
        value: Arc<dyn Any + Send + Sync>,
    ) -> Self::ContextId;

    fn get_value(&self, context_id: Self::ContextId, val_id: ValueId)
    -> Arc<dyn Any + Send + Sync>;

    fn get_values(
        &self,
        context_id: Self::ContextId,
        val_ids: &[ValueId],
    ) -> Vec<Arc<dyn Any + Send + Sync>>;

    fn remove_context(&self, context_id: Self::ContextId);
}
