use crate::kernel::CallSpec;

pub trait Scheduler: Send + Sync {
    type Route: Send + Sync + Clone + 'static;
    fn route(&self, call: &CallSpec) -> Self::Route;
}
