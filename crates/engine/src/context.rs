use std::{any::Any, cmp::Ordering, sync::Arc};

use namu_core::ir::ValueId;

pub mod dynamic_context;
pub mod naive_context;
pub mod static_context;

pub trait ContextManager {
    type ContextId;

    fn create_context(&self) -> Self::ContextId;

    fn compare_context(&self, a: Self::ContextId, b: Self::ContextId) -> Ordering;

    fn add_variable(
        &self,
        context_id: Self::ContextId,
        var_id: ValueId,
        value: Arc<dyn Any + Send + Sync>,
    ) -> Self::ContextId;

    fn get_variable(
        &self,
        context_id: Self::ContextId,
        var_id: ValueId,
    ) -> Arc<dyn Any + Send + Sync>;

    fn get_variables(
        &self,
        context_id: Self::ContextId,
        var_ids: &[ValueId],
    ) -> Vec<Arc<dyn Any + Send + Sync>>;

    fn remove_context(&self, context_id: Self::ContextId);
}
