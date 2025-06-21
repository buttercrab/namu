use std::sync::atomic::{AtomicUsize, Ordering};

use namu_core::{Value, ValueId};
use scc::HashMap;

use crate::context::ContextManager;

pub struct NaiveContextManager {
    contexts: HashMap<usize, HashMap<ValueId, Value>>,
    context_order: HashMap<usize, Vec<usize>>,
    id_counter: AtomicUsize,
}

impl NaiveContextManager {
    pub fn new() -> Self {
        Self {
            contexts: HashMap::new(),
            context_order: HashMap::new(),
            id_counter: AtomicUsize::new(0),
        }
    }
}

impl ContextManager for NaiveContextManager {
    type ContextId = usize;

    fn create_context(&self) -> Self::ContextId {
        let id = self.id_counter.fetch_add(1, Ordering::Relaxed);
        self.contexts.insert(id, HashMap::new()).unwrap();
        self.context_order.insert(id, vec![id]).unwrap();
        id
    }

    fn compare_context(&self, a: Self::ContextId, b: Self::ContextId) -> std::cmp::Ordering {
        let a_order = self.context_order.get(&a).unwrap();
        let b_order = self.context_order.get(&b).unwrap();
        a_order.cmp(&b_order)
    }

    fn add_value(
        &self,
        context_id: Self::ContextId,
        val_id: ValueId,
        value: Value,
    ) -> Self::ContextId {
        let context = self.contexts.get(&context_id).unwrap().clone();
        let mut context_order = self.context_order.get(&context_id).unwrap().clone();
        context.insert(val_id, value).unwrap();
        let id = self.id_counter.fetch_add(1, Ordering::Relaxed);
        context_order.push(id);
        self.contexts.insert(id, context).unwrap();
        self.context_order.insert(id, context_order).unwrap();
        id
    }

    fn get_value(&self, context_id: Self::ContextId, val_id: ValueId) -> Value {
        let context = self.contexts.get(&context_id).unwrap();
        context.get().get(&val_id).unwrap().clone()
    }

    fn get_values(&self, context_id: Self::ContextId, val_ids: &[ValueId]) -> Vec<Value> {
        let context = self.contexts.get(&context_id).unwrap();
        val_ids
            .iter()
            .map(|val_id| context.get().get(val_id).unwrap().clone())
            .collect()
    }

    fn remove_context(&self, context_id: Self::ContextId) {
        self.contexts.remove(&context_id);
        self.context_order.remove(&context_id);
    }
}
