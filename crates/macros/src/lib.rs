extern crate proc_macro;

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{
    parse_macro_input,
    visit_mut::{self, VisitMut},
    Expr, FnArg, Ident, ItemFn, Pat, ReturnType, Type,
};

#[proc_macro_attribute]
pub fn workflow(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let mut input_fn = parse_macro_input!(item as ItemFn);

    struct WorkflowVisitor;
    impl VisitMut for WorkflowVisitor {
        fn visit_expr_mut(&mut self, expr: &mut Expr) {
            // Recurse first to transform nested expressions
            visit_mut::visit_expr_mut(self, expr);

            if let Expr::If(if_expr) = expr {
                let cond = &if_expr.cond;
                let then_branch = &if_expr.then_branch;

                if let Some((_, else_branch)) = &if_expr.else_branch {
                    let new_code = quote! {
                        graph::graph_if(#cond, #then_branch, #else_branch)
                    };

                    if let Ok(new_expr) = syn::parse2(new_code.clone()) {
                        *expr = new_expr;
                    } else {
                        let error = syn::Error::new_spanned(
                            &mut *expr,
                            format!(
                                "Failed to transform 'if' expression. Generated code was: {}",
                                new_code
                            ),
                        );
                        *expr = syn::parse2(error.to_compile_error()).unwrap();
                    }
                }
            }
        }
    }

    let mut visitor = WorkflowVisitor;
    visitor.visit_block_mut(&mut input_fn.block);

    TokenStream::from(quote! { #input_fn })
}

#[proc_macro_attribute]
pub fn trace(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let mut input_fn = parse_macro_input!(item as ItemFn);
    let original_fn = input_fn.clone();
    let original_fn_name = &original_fn.sig.ident;
    let new_original_fn_name = format_ident!("__trace_original_{}", original_fn_name);

    let mut renamed_fn = original_fn.clone();
    renamed_fn.sig.ident = new_original_fn_name.clone();

    let fn_name = &input_fn.sig.ident;
    let fn_name_str = fn_name.to_string();
    let visibility = &input_fn.vis;

    let original_return_type = match &input_fn.sig.output {
        ReturnType::Default => quote! { () },
        ReturnType::Type(_, ty) => quote! { #ty },
    };

    let arg_types: Vec<Box<Type>> = input_fn
        .sig
        .inputs
        .iter()
        .map(|arg| {
            if let FnArg::Typed(pat_type) = arg {
                return pat_type.ty.clone();
            }
            panic!("Unsupported argument pattern");
        })
        .collect();

    let arg_names: Vec<Ident> = input_fn
        .sig
        .inputs
        .iter_mut()
        .map(|arg| {
            if let FnArg::Typed(pat_type) = arg {
                let ty = &pat_type.ty;
                if let Type::Path(type_path) = &**ty {
                    if let Some(segment) = type_path.path.segments.last() {
                        if segment.ident != "TraceNode" {
                            pat_type.ty = Box::new(
                                syn::parse_str(&quote! { graph::TraceNode<#ty> }.to_string())
                                    .unwrap(),
                            );
                        }
                    }
                } else {
                    pat_type.ty = Box::new(
                        syn::parse_str(&quote! { graph::TraceNode<#ty> }.to_string()).unwrap(),
                    );
                }

                if let Pat::Ident(pat_ident) = &*pat_type.pat {
                    return pat_ident.ident.clone();
                }
            }
            panic!("Unsupported argument pattern");
        })
        .collect();

    input_fn.sig.output =
        syn::parse_str(&quote! { -> graph::TraceNode<#original_return_type> }.to_string()).unwrap();

    let sig = &input_fn.sig;

    let downcast_args = arg_types.iter().enumerate().map(|(i, ty)| {
        let arg_name = format_ident!("arg_{}", i);
        quote! {
            let #arg_name = inputs[#i].downcast_ref::<#ty>().unwrap();
        }
    });

    let arg_passing = arg_names.iter().enumerate().map(|(i, _)| {
        let arg_name = format_ident!("arg_{}", i);
        quote! { *#arg_name }
    });

    let wrapper_body = if arg_names.is_empty() {
        quote! {
            let func = std::sync::Arc::new(|_inputs: Vec<graph::Value>| {
                let result = #new_original_fn_name();
                std::sync::Arc::new(result) as graph::Value
            });
            graph::new_call(#fn_name_str, func, vec![])
        }
    } else {
        quote! {
            let func = std::sync::Arc::new(|inputs: Vec<graph::Value>| {
                #(#downcast_args)*
                let result = #new_original_fn_name(#(#arg_passing),*);
                std::sync::Arc::new(result) as graph::Value
            });

            let parents = vec![#(#arg_names.node),*];

            graph::new_call(#fn_name_str, func, parents)
        }
    };

    let expanded = quote! {
        #renamed_fn

        #visibility #sig {
            #wrapper_body
        }
    };

    TokenStream::from(expanded)
}
