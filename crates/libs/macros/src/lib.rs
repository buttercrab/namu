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
/// It describes the type too.
#[proc_macro_derive(Data)]
pub fn derive_data(input: TokenStream) -> TokenStream {
    data::derive_data(input)
}

/// Describes a type's shape, for signatures: `#[derive(Serialize, Deserialize, Describe)]`.
#[proc_macro_derive(Describe)]
pub fn derive_describe(input: TokenStream) -> TokenStream {
    data::derive_describe(input, quote::quote! { ::namu::__macro_exports })
}

/// `Describe` for task crates, re-exported by `namu_guest` as its `Describe`.
#[doc(hidden)]
#[proc_macro_derive(GuestDescribe)]
pub fn derive_guest_describe(input: TokenStream) -> TokenStream {
    data::derive_describe(input, quote::quote! { ::namu_guest::__private })
}
