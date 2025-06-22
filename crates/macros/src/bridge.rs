use darling::FromMeta;
use darling::ast::NestedMeta;
use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::punctuated::Punctuated;
use syn::{Token, Type, parse_macro_input};

#[derive(Debug, FromMeta)]
struct Args {
    fn_name: syn::Ident,
    task: syn::Path,
    inputs: Option<syn::Type>,
    output: syn::Type,
    name: Option<String>,
    author: Option<String>,
    version: Option<String>,
}

pub fn task_bridge_impl(input: TokenStream) -> TokenStream {
    let meta: Punctuated<NestedMeta, Token![,]> =
        parse_macro_input!(input with Punctuated::<NestedMeta, Token![,]>::parse_terminated);
    let meta_vec: Vec<NestedMeta> = meta.into_iter().collect();
    let args = match Args::from_list(&meta_vec) {
        Ok(v) => v,
        Err(e) => return e.write_errors().into(),
    };

    let fn_name_ident = &args.fn_name;
    let task_path = &args.task;
    let inputs_type_opt = args.inputs;
    let output_type = &args.output;

    // Derive input types vector and parameter list
    let (input_types, param_tokens, id_vec_tokens, pack_param_names): (
        Vec<Type>,
        proc_macro2::TokenStream,
        proc_macro2::TokenStream,
        Vec<syn::Ident>,
    ) = if let Some(inputs_type) = inputs_type_opt.clone() {
        if let Type::Tuple(tuple) = &inputs_type {
            let tys: Vec<_> = tuple.elems.iter().cloned().collect();
            let mut param_tokens = proc_macro2::TokenStream::new();
            let mut id_vec = proc_macro2::TokenStream::new();
            let mut names = Vec::new();
            for (idx, ty) in tys.iter().enumerate() {
                let name = format_ident!("arg{idx}");
                names.push(name.clone());
                param_tokens.extend(quote! { #name: ::namu::__macro_exports::TracedValue<#ty>, });
                id_vec.extend(quote! { #name.id, });
            }
            (tys, param_tokens, id_vec, names)
        } else {
            // single type
            let ty = inputs_type;
            let name = format_ident!("arg0");
            let param_tokens = quote! { #name: ::namu::__macro_exports::TracedValue<#ty>, };
            let id_vec = quote! { #name.id };
            (vec![ty], param_tokens, id_vec, vec![name])
        }
    } else {
        // zero inputs
        (
            Vec::new(),
            proc_macro2::TokenStream::new(),
            proc_macro2::TokenStream::new(),
            Vec::new(),
        )
    };

    // Determine call fn ident based on output tuple size
    let (ret_tokens, call_fn_ident) = if let Type::Tuple(tuple) = output_type {
        if tuple.elems.is_empty() {
            // ()
            (
                quote! { ::namu::__macro_exports::TracedValue<()> },
                format_ident!("call"),
            )
        } else {
            let arity = tuple.elems.len();
            let traced_tuple: Vec<_> = tuple
                .elems
                .iter()
                .map(|ty| quote! { ::namu::__macro_exports::TracedValue<#ty> })
                .collect();
            let ret = quote! { ( #( #traced_tuple ),* ) };
            (ret, format_ident!("call{}", arity))
        }
    } else {
        (
            quote! { ::namu::__macro_exports::TracedValue<#output_type> },
            format_ident!("call"),
        )
    };

    // Build pack function tokens (simple: if >1 inputs create tuple)
    let pack_fn_tokens = {
        let pack_fn_ident = format_ident!("pack");
        let arg_len = input_types.len();
        if arg_len == 0 {
            quote! {
                #[allow(dead_code)]
                pub fn #pack_fn_ident(_inputs: Vec<::namu::__macro_exports::Value>) -> ::namu::__macro_exports::Value {
                    ::namu::__macro_exports::Value::new(())
                }
            }
        } else if arg_len == 1 {
            quote! {
                #[allow(dead_code)]
                pub fn #pack_fn_ident(mut inputs: Vec<::namu::__macro_exports::Value>) -> ::namu::__macro_exports::Value {
                    debug_assert_eq!(inputs.len(), 1);
                    inputs.pop().unwrap()
                }
            }
        } else {
            let vars: Vec<_> = pack_param_names.iter().collect();
            let downcasts = vars.iter().zip(input_types.iter()).map(|(v, ty)| {
                quote! {
                    let #v = {
                        let val = inputs.remove(0);
                        (*val.downcast_ref::<#ty>().expect("pack downcast failed")).clone()
                    };
                }
            });
            let tuple_tokens = quote! { (#( #vars ),*) };
            quote! {
                #[allow(dead_code)]
                pub fn #pack_fn_ident(mut inputs: Vec<::namu::__macro_exports::Value>) -> ::namu::__macro_exports::Value {
                    debug_assert_eq!(inputs.len(), #arg_len);
                    #(#downcasts)*
                    ::namu::__macro_exports::Value::new(#tuple_tokens)
                }
            }
        }
    };

    // Build unpack function tokens
    let unpack_fn_tokens = {
        let unpack_fn_ident = format_ident!("unpack");
        if let Type::Tuple(tuple) = output_type {
            if tuple.elems.is_empty() {
                // returns ()
                quote! {
                    #[allow(dead_code)]
                    pub fn #unpack_fn_ident(val: ::namu::__macro_exports::Value) -> Vec<::namu::__macro_exports::Value> {
                        vec![val]
                    }
                }
            } else {
                let arity = tuple.elems.len();
                let vars: Vec<_> = (0..arity).map(|i| format_ident!("o{i}")).collect();
                quote! {
                    #[allow(dead_code)]
                    pub fn #unpack_fn_ident(val: ::namu::__macro_exports::Value) -> Vec<::namu::__macro_exports::Value> {
                        let tuple_val: #output_type = unsafe { val.take::<#output_type>() };
                        let ( #( #vars ),* ) = tuple_val;
                        vec![ #( ::namu::__macro_exports::Value::new(#vars) ),* ]
                    }
                }
            }
        } else {
            quote! {
                #[allow(dead_code)]
                pub fn #unpack_fn_ident(val: ::namu::__macro_exports::Value) -> Vec<::namu::__macro_exports::Value> {
                    vec![val]
                }
            }
        }
    };

    // Build optional inventory submission
    let inventory_tokens = if let (Some(name_lit), Some(author_lit)) = (args.name, args.author) {
        let version_lit = args.version.unwrap_or_else(|| "0.1".to_string());
        quote! {
            ::namu::__macro_exports::inventory::submit! {
                ::namu::__macro_exports::TaskEntry {
                    name:   #name_lit,
                    author: #author_lit,
                    create: || Box::new(<#task_path as ::core::default::Default>::default()),
                    pack:   Some(pack),
                    unpack: Some(unpack),
                    version: #version_lit,
                }
            }
        }
    } else {
        quote! {}
    };

    // Generate wrapper module with builder function
    let fn_builder_tokens = quote! {
        pub fn #fn_name_ident<G: 'static>(builder: &::namu::__macro_exports::Builder<G>, #param_tokens) -> #ret_tokens {
            ::namu::__macro_exports::#call_fn_ident(&builder, stringify!(#fn_name_ident), vec![#id_vec_tokens])
        }
    };

    let expanded = quote! {
        #[allow(non_snake_case)]
        pub mod #fn_name_ident {
            use super::*;
            #pack_fn_tokens
            #unpack_fn_tokens
            #inventory_tokens
        }

        #fn_builder_tokens
    };

    expanded.into()
}
