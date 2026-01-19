use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{DeriveInput, parse_macro_input};

pub fn r#type(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as DeriveInput);
    let name = &input.ident;
    let type_name = name.to_string();
    let deser_fn = format_ident!("__namu_deserialize_{}", name.to_string().to_lowercase());

    let expanded = quote! {
        #input

        #[allow(non_camel_case_types)]
        fn #deser_fn(
            de: &mut dyn erased_serde::Deserializer,
        ) -> Result<::namu::__macro_exports::Value, erased_serde::Error> {
            let t = erased_serde::deserialize::<#name>(de)?;
            Ok(::namu::__macro_exports::Value::new(t))
        }

        ::namu::__macro_exports::inventory::submit! {
            ::namu::__macro_exports::TypeEntry {
                name: #type_name,
                type_id: stringify!(#name),
                deserialize: #deser_fn,
            }
        }
    };
    TokenStream::from(expanded)
}
