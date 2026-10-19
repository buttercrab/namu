//! `#[derive(Data)]`: a type the engine holds as an opaque value (design §8.2). Every type
//! parameter must be `Data` too.

use proc_macro::TokenStream;
use quote::quote;
use syn::{DeriveInput, parse_macro_input, parse_quote};

pub fn derive(input: TokenStream) -> TokenStream {
    let mut input = parse_macro_input!(input as DeriveInput);
    for param in input.generics.type_params_mut() {
        param.bounds.push(parse_quote!(::namu::Data));
    }
    let name = &input.ident;
    let (impl_generics, type_generics, where_clause) = input.generics.split_for_impl();
    quote! {
        impl #impl_generics ::namu::Data for #name #type_generics #where_clause {}
    }
    .into()
}
