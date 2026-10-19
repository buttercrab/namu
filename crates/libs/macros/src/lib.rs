use proc_macro::TokenStream;

mod data;
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

/// Lets the engine hold a type as a value: `#[derive(Clone, Serialize, Deserialize, Data)]`.
#[proc_macro_derive(Data)]
pub fn derive_data(input: TokenStream) -> TokenStream {
    data::derive(input)
}
