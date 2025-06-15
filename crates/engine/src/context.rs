use std::{any::Any, cmp::Ordering, sync::Arc};

use common::VarId;

mod dynamic;
mod naive;

pub trait ContextManager {
    type ContextId;

    fn create_context(&self) -> Self::ContextId;

    fn compare_context(&self, a: Self::ContextId, b: Self::ContextId) -> Ordering;

    fn add_variable(
        &self,
        context_id: Self::ContextId,
        var_id: VarId,
        value: Arc<dyn Any + Send + Sync>,
    ) -> Self::ContextId;

    fn get_variable(
        &self,
        context_id: Self::ContextId,
        var_id: VarId,
    ) -> Arc<dyn Any + Send + Sync>;

    fn get_variables(
        &self,
        context_id: Self::ContextId,
        var_ids: &[VarId],
    ) -> Vec<Arc<dyn Any + Send + Sync>>;

    fn remove_context(&self, context_id: Self::ContextId);
}
