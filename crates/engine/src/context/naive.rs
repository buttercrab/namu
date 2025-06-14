use std::{any::Any, cell::RefCell, collections::HashMap, sync::Arc};

use common::VarId;

use crate::context::ContextManager;

struct NaiveContextManagerInner {
    contexts: HashMap<usize, HashMap<VarId, Arc<dyn Any + Send + Sync>>>,
    context_order: HashMap<usize, Vec<usize>>,
    id_counter: usize,
}

pub struct NaiveContextManager {
    inner: RefCell<NaiveContextManagerInner>,
}

impl NaiveContextManager {
    pub fn new() -> Self {
        Self {
            inner: RefCell::new(NaiveContextManagerInner {
                contexts: HashMap::new(),
                context_order: HashMap::new(),
                id_counter: 0,
            }),
        }
    }
}

impl ContextManager for NaiveContextManager {
    type ContextId = usize;

    fn create_context(&self) -> Self::ContextId {
        let mut inner = self.inner.borrow_mut();
        let id = inner.id_counter;
        inner.id_counter += 1;
        inner.contexts.insert(id, HashMap::new());
        inner.context_order.insert(id, vec![id]);
        id
    }

    fn compare_context(&self, a: Self::ContextId, b: Self::ContextId) -> std::cmp::Ordering {
        let inner = self.inner.borrow();
        let a_order = inner.context_order.get(&a).unwrap();
        let b_order = inner.context_order.get(&b).unwrap();
        a_order.cmp(b_order)
    }

    fn add_variable(
        &self,
        context_id: Self::ContextId,
        var_id: common::VarId,
        value: Arc<dyn Any + Send + Sync>,
    ) -> Self::ContextId {
        let inner = self.inner.borrow();
        let mut context = inner.contexts.get(&context_id).unwrap().clone();
        let mut context_order = inner.context_order.get(&context_id).unwrap().clone();
        drop(inner);
        context.insert(var_id, value);
        let mut inner = self.inner.borrow_mut();
        let id = inner.id_counter;
        context_order.push(id);
        inner.id_counter += 1;
        inner.contexts.insert(id, context);
        inner.context_order.insert(id, context_order);
        id
    }

    fn get_variable(
        &self,
        context_id: Self::ContextId,
        var_id: common::VarId,
    ) -> Option<Arc<dyn Any + Send + Sync>> {
        let inner = self.inner.borrow();
        let context = inner.contexts.get(&context_id).unwrap();
        context.get(&var_id).cloned()
    }

    fn get_variables(
        &self,
        context_id: Self::ContextId,
        var_ids: &[common::VarId],
    ) -> Vec<Arc<dyn Any + Send + Sync>> {
        let inner = self.inner.borrow();
        let context = inner.contexts.get(&context_id).unwrap();
        var_ids
            .iter()
            .map(|var_id| context.get(var_id).unwrap())
            .cloned()
            .collect()
    }

    fn remove_context(&self, context_id: Self::ContextId) {
        let mut inner = self.inner.borrow_mut();
        inner.contexts.remove(&context_id);
        inner.context_order.remove(&context_id);
    }
}
