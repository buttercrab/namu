use darling::{FromMeta, ast::NestedMeta};
use proc_macro::TokenStream;
use proc_macro_error::abort;
use quote::{format_ident, quote};
use syn::{
    AngleBracketedGenericArguments, GenericArgument, ItemFn, Pat, Path, PathArguments, PathSegment,
    ReturnType, Type, TypePath, parse_macro_input, parse_quote,
};

#[derive(Debug, FromMeta)]
struct TaskArgs {
    #[darling(default)]
    r#type: TaskType,
}

#[derive(Debug, Default, FromMeta, PartialEq)]
#[darling(rename_all = "snake_case")]
enum TaskType {
    #[default]
    Single,
    Batch,
    Stream,
}

pub fn task(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr_args = match NestedMeta::parse_meta_list(attr.into()) {
        Ok(v) => v,
        Err(e) => {
            return TokenStream::from(darling::Error::from(e).write_errors());
        }
    };
    let args = match TaskArgs::from_list(&attr_args) {
        Ok(v) => v,
        Err(e) => {
            return TokenStream::from(e.write_errors());
        }
    };

    let func = parse_macro_input!(item as ItemFn);
    let func_name = &func.sig.ident;
    let struct_name = format_ident!("__{}", func_name);

    // Keep a renamed version of the original function for the implementation
    let task_impl_func = {
        let mut f = func.clone();
        f.sig.ident = format_ident!("__impl_{}", func_name);
        quote! { #f }
    };
    let task_impl_func_name = format_ident!("__impl_{}", func_name);

    // --- Argument & Return Type Analysis ---
    let mut arg_names = Vec::new();
    let mut arg_types = Vec::new();
    for arg in func.sig.inputs.iter() {
        let syn::FnArg::Typed(pt) = arg else {
            abort!(arg, "`self` is not supported");
        };
        let Pat::Ident(pat_ident) = &*pt.pat else {
            abort!(pt, "only simple idents are supported");
        };
        arg_names.push(pat_ident.ident.clone());
        arg_types.push(pt.ty.clone());
    }

    let (input_type, input_packing) = if arg_types.len() == 1 {
        let ty = &arg_types[0];
        let name = &arg_names[0];
        (quote! { #ty }, quote! { #name })
    } else {
        (quote! { (#(#arg_types),*) }, quote! { (#(#arg_names),*) })
    };

    let ReturnType::Type(_, ty) = &func.sig.output else {
        abort!(func.sig.output, "the function should return `Result`");
    };

    let Type::Path(TypePath {
        qself: None,
        path:
            Path {
                leading_colon: None,
                segments: result_segments,
            },
    }) = &**ty
    else {
        abort!(func.sig.output, "the function should return `Result`");
    };

    let PathSegment {
        ident,
        arguments:
            PathArguments::AngleBracketed(AngleBracketedGenericArguments {
                args: result_args, ..
            }),
    } = result_segments.last().unwrap()
    else {
        abort!(func.sig.output, "the function should return `Result`");
    };

    if ident != "Result" {
        abort!(func.sig.output, "the function should return `Result`");
    }

    let GenericArgument::Type(output_type) = result_args.first().unwrap() else {
        abort!(func.sig.output, "the function should return `Result`");
    };

    // --- Generate Trait Implementation ---
    let task_trait_impl = match args.r#type {
        TaskType::Single => {
            let input_destructuring = if arg_names.len() == 1 {
                let name = &arg_names[0];
                quote! { let #name = input; }
            } else {
                quote! { let (#(#arg_names),*) = input; }
            };
            quote! {
                #[allow(non_camel_case_types)]
                struct #struct_name;
                impl task::Task for #struct_name {
                    type Config = ();
                    type Input = #input_type;
                    type Output = #output_type;
                    fn new(_config: Self::Config) -> Self { Self }
                    fn run(&mut self, recv: task::Receiver<(usize, Self::Input)>, send: task::Sender<(usize, anyhow::Result<Self::Output>)>) {
                        task::SingleTask::run(self, recv, send);
                    }
                }
                impl task::SingleTask for #struct_name {
                    fn call(&mut self, input: Self::Input) -> anyhow::Result<Self::Output> {
                        #input_destructuring
                        let result = #task_impl_func_name(#(#arg_names),*)?;
                        Ok(result)
                    }
                }
            }
        }
        TaskType::Batch => {
            if arg_names.len() != 1 {
                abort!(
                    func.sig.inputs,
                    "Batch task must have exactly one argument: Vec<Input>"
                );
            }
            quote! {
                struct #struct_name;
                impl task::Task for #struct_name {
                    type Config = ();
                    type Input = #input_type;
                    type Output = #output_type;
                    fn new(_config: Self::Config) -> Self { Self }
                    fn run(&mut self, recv: task::Receiver<(usize, Self::Input)>, send: task::Sender<(usize, anyhow::Result<Self::Output>)>) {
                        task::BatchedTask::run(self, recv, send);
                    }
                }
                impl task::BatchedTask for #struct_name {
                    fn batch_size(&self) -> usize { 16 } // Default, could be an attribute
                    fn call(&mut self, input: Vec<Self::Input>) -> Vec<anyhow::Result<Self::Output>> {
                        #task_impl_func_name(input)
                    }
                }
            }
        }
        TaskType::Stream => {
            if arg_names.len() != 1 {
                abort!(
                    func.sig.inputs,
                    "Stream task must have exactly one argument: Input"
                );
            }
            quote! {
                #[allow(non_camel_case_types)]
                struct #struct_name;
                impl task::Task for #struct_name {
                    type Config = ();
                    type Input = #input_type;
                    type Output = #output_type;
                    fn new(_config: Self::Config) -> Self { Self }
                    fn run(&mut self, recv: task::Receiver<(usize, Self::Input)>, send: task::Sender<(usize, anyhow::Result<Self::Output>)>) {
                        task::StreamTask::run(self, recv, send);
                    }
                }
                impl task::StreamTask for #struct_name {
                    fn call(&mut self, input: Self::Input) -> impl Iterator<Item = anyhow::Result<Self::Output>> {
                        #task_impl_func_name(input)
                    }
                }
            }
        }
    };

    // --- Generate Constructor for Graph Builder (Backward Compatible) ---
    let mut constructor_sig = func.sig.clone();
    constructor_sig.generics = parse_quote! { <T: Clone + 'static> };
    constructor_sig.inputs.clear();
    constructor_sig
        .inputs
        .push(parse_quote! { builder: &graph::Builder<T> });
    for (name, ty) in arg_names.iter().zip(arg_types.iter()) {
        constructor_sig
            .inputs
            .push(parse_quote! { #name: graph::TracedValue<#ty> });
    }
    constructor_sig.output = parse_quote! { -> graph::TracedValue<#output_type> };

    let value_downcasts = arg_names.iter().enumerate().map(|(i, name)| {
        let ty = &arg_types[i];
        quote! { let #name = __inputs[#i].downcast_ref::<#ty>().unwrap().clone(); }
    });
    let input_ids = arg_names.iter().map(|name| quote! { #name.id });

    // A unique, static-friendly identifier for the task
    let task_id_str = format!("{}::{}", func.sig.ident, file!());

    let registrar_name = format_ident!("__factory_{}", func_name);
    let static_registrar_name = format_ident!("__REG_ONCE_{}", func_name);

    // todo: change factory logic
    let factory_logic = match args.r#type {
        TaskType::Single => quote! {
            std::sync::Arc::new(|__inputs| {
                #(#value_downcasts)*
                let mut task_instance = #struct_name;
                // todo: change unwrap
                let result = task::SingleTask::call(&mut task_instance, #input_packing).unwrap();
                std::sync::Arc::new(result) as graph::Value
            })
        },
        _ => quote! {
            panic!("Task type not supported by the local graph::run() executor");
        },
    };

    let constructor = quote! {
        // This function creates a factory which in turn creates the executable closure.
        fn #registrar_name() -> graph::TaskFactory {
            std::sync::Arc::new(|| {
                #factory_logic
            })
        }

        #[allow(non_snake_case)]
        pub #constructor_sig {
            // Ensure this task is registered in the global registry, but only once.
            #[allow(non_upper_case_globals)]
            static #static_registrar_name: std::sync::Once = std::sync::Once::new();
            #static_registrar_name.call_once(|| {
                graph::register_task(#task_id_str.to_string(), #registrar_name());
            });

            let kind = graph::NodeKind::Call {
                name: stringify!(#func_name),
                task_id: #task_id_str.to_string(),
                inputs: vec![#(#input_ids),*],
            };
            builder.add_instruction(kind)
        }
    };

    // --- Assemble Final Token Stream ---
    quote! {
        #task_impl_func
        #task_trait_impl
        #constructor
    }
    .into()
}
