use proc_macro::TokenStream;

mod task;
mod workflow;

/// Declares a task by its signature; see the `task` module.
#[proc_macro_attribute]
pub fn task(attr: TokenStream, item: TokenStream) -> TokenStream {
    task::task(attr, item)
}

/// Compiles a workflow function to IR; see the `workflow` module.
#[proc_macro_attribute]
pub fn workflow(attr: TokenStream, item: TokenStream) -> TokenStream {
    workflow::workflow(attr, item)
}
