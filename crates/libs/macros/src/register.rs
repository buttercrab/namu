use darling::FromMeta;
use darling::ast::NestedMeta;
use proc_macro::TokenStream;
use quote::quote;
use syn::punctuated::Punctuated;
use syn::{Token, parse_macro_input};

#[derive(Debug, FromMeta)]
struct Args {
    method: syn::Path,
    name: String,
    author: String,
    version: Option<String>,
}

pub fn register_task_impl(input: TokenStream) -> TokenStream {
    let meta: Punctuated<NestedMeta, Token![,]> =
        parse_macro_input!(input with Punctuated::<NestedMeta, Token![,]>::parse_terminated);
    let meta_vec: Vec<NestedMeta> = meta.into_iter().collect();
    let args = match Args::from_list(&meta_vec) {
        Ok(v) => v,
        Err(e) => return e.write_errors().into(),
    };

    let method_path = args.method;
    let name_lit = args.name;
    let author_lit = args.author;
    let version_lit = args.version.unwrap_or_else(|| "0.1".to_string());

    let expanded = quote! {
        ::namu::__macro_exports::inventory::submit! {
            ::namu::__macro_exports::TaskEntry {
                name:   #name_lit,
                author: #author_lit,
                create: || Box::new(#method_path::Task),
                pack:   Some(#method_path::pack),
                unpack: Some(#method_path::unpack),
                version: #version_lit,
            }
        }
    };

    expanded.into()
}
