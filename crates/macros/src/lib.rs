use proc_macro::TokenStream;
use proc_macro_error::proc_macro_error;

mod register;
mod task;
mod workflow;

#[proc_macro_error]
#[proc_macro_attribute]
pub fn task(attr: TokenStream, item: TokenStream) -> TokenStream {
    task::task(attr, item)
}

#[proc_macro_error]
#[proc_macro_attribute]
pub fn workflow(attr: TokenStream, item: TokenStream) -> TokenStream {
    workflow::workflow(attr, item)
}

#[proc_macro]
pub fn register_task(item: TokenStream) -> TokenStream {
    register::register_task_impl(item)
}
