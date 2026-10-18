use std::sync::atomic::{AtomicUsize, Ordering};

use namu_core::{ContextId, Value, ValueId};
use scc::HashMap;

use crate::context::ContextManager;

#[derive(Default, Debug)]
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
    fn create_context(&self) -> ContextId {
        let id = self.id_counter.fetch_add(1, Ordering::Relaxed);
        self.contexts.insert_sync(id, HashMap::new()).unwrap();
        self.context_order.insert_sync(id, vec![id]).unwrap();
        id
    }

    fn compare_context(&self, a: ContextId, b: ContextId) -> std::cmp::Ordering {
        let a_order = self.context_order.get_sync(&a).unwrap();
        let b_order = self.context_order.get_sync(&b).unwrap();
        a_order.cmp(&b_order)
    }

    fn add_value(&self, context_id: ContextId, val_id: ValueId, value: Value) -> ContextId {
        let context = self.contexts.get_sync(&context_id).unwrap().clone();
        let mut context_order = self.context_order.get_sync(&context_id).unwrap().clone();
        context.insert_sync(val_id, value).unwrap();
        let id = self.id_counter.fetch_add(1, Ordering::Relaxed);
        context_order.push(id);
        self.contexts.insert_sync(id, context).unwrap();
        self.context_order.insert_sync(id, context_order).unwrap();
        id
    }

    fn get_value(&self, context_id: ContextId, val_id: ValueId) -> Value {
        let context = self.contexts.get_sync(&context_id).unwrap();
        context.get().get_sync(&val_id).unwrap().get().clone()
    }

    fn get_values(&self, context_id: ContextId, val_ids: &[ValueId]) -> Vec<Value> {
        let context = self.contexts.get_sync(&context_id).unwrap();
        val_ids
            .iter()
            .map(|val_id| context.get().get_sync(val_id).unwrap().get().clone())
            .collect()
    }

    fn remove_context(&self, context_id: ContextId) {
        self.contexts.remove_sync(&context_id);
        self.context_order.remove_sync(&context_id);
    }
}
