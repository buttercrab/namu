//! The `#[task]` procedural macro.
//!
//! This macro is the primary way to define a computational task. It transforms
//! a standard Rust function into a struct that implements the `task::Task`
//! and `task::SingleTask` (or `BatchedTask`/`StreamTask`) traits.
//!
//! ## Macro Arguments
//! The macro can be used in several ways:
//!   - `#[task]`: Defines a `SingleTask`. This is the default.
//!   - `#[task(batch)]`: Defines a `BatchedTask` with a default batch size.
//!   - `#[task(batch, batch_size = 16)]`: Defines a `BatchedTask` with a specific batch size.
//!   - `#[task(stream)]`: Defines a `StreamTask`.
//!
//! ## Generated Code
//! 1.  **Renamed Original Function**: The user's function is preserved with a
//!     prefix (e.g., `__impl_my_task`).
//! 2.  **Task Struct**: A unit struct (e.g., `__my_task`) is created to serve as
//!     the task's identity and method container.
//! 3.  **`task::Task` Implementation**: An `impl<Id, C> task::Task<Id, C> for ...`
//!     block is generated. This contains the `prepare` and `run` methods. The
//!     `run` logic is taken from the default implementations on the specialized
//!     traits (`SingleTask`, etc.).
//! 4.  **Specialized Trait Implementation**: An `impl` for `task::SingleTask`,
//!     `task::BatchedTask`, or `task::StreamTask` is created. This defines the
//!     `Input` and `Output` associated types and implements the `call` method,
//!     which invokes the renamed original function.
//! 5.  **Graph Constructor**: A function with the same name as the original is
//!     generated. This function is what's called inside a `#[workflow]`. It
//!     takes `TracedValue`s as input, registers the task with the executor's
//!     registry, and adds a `Call` node to the graph via the `Builder` API.

use proc_macro::TokenStream;
use proc_macro_error::abort;
use quote::{format_ident, quote};
use syn::{
    AngleBracketedGenericArguments, GenericArgument, Ident, ItemFn, LitInt, Pat, Path,
    PathArguments, PathSegment, ReturnType, Token, Type, TypePath,
    parse::{Parse, ParseStream},
    parse_macro_input, parse_quote,
};

// --- Attribute Parsing ---

#[derive(Debug, Default)]
enum TaskType {
    #[default]
    Single,
    Batch,
    Stream,
}

#[derive(Debug, Default)]
struct TaskArgs {
    task_type: TaskType,
    batch_size: Option<usize>,
}

impl Parse for TaskArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut args = TaskArgs::default();
        if input.is_empty() {
            return Ok(args);
        }

        let task_type_ident: Ident = input.parse()?;
        match task_type_ident.to_string().as_str() {
            "single" => args.task_type = TaskType::Single,
            "batch" => args.task_type = TaskType::Batch,
            "stream" => args.task_type = TaskType::Stream,
            _ => {
                return Err(syn::Error::new(
                    task_type_ident.span(),
                    "expected `single`, `batch`, or `stream`",
                ));
            }
        }

        if !input.is_empty() {
            input.parse::<Token![,]>()?;
        }

        // Check for optional `batch_size`
        if !input.is_empty() {
            let lookahead = input.lookahead1();
            if lookahead.peek(Ident) {
                let ident: Ident = input.parse()?;
                if ident == "batch_size" {
                    input.parse::<Token![=]>()?;
                    let lit: LitInt = input.parse()?;
                    args.batch_size = Some(lit.base10_parse()?);
                }
            }
        }

        Ok(args)
    }
}

// --- Type-parsing Helpers ---

/// Extracts the inner type `T` from a `Result<T, E>`.
fn extract_result_type(ty: &Type) -> &Type {
    let Type::Path(TypePath {
        qself: None,
        path: Path {
            leading_colon: None,
            segments,
        },
    }) = ty
    else {
        abort!(ty, "Task function must return a `Result`");
    };

    let Some(PathSegment {
        ident,
        arguments: PathArguments::AngleBracketed(AngleBracketedGenericArguments { args, .. }),
    }) = segments.last()
    else {
        abort!(ty, "Task function must return a `Result`");
    };

    if ident != "Result" {
        abort!(ty, "Task function must return a `Result`");
    }

    let Some(GenericArgument::Type(inner_type)) = args.first() else {
        abort!(ty, "Result must have a type argument, e.g., Result<i32>");
    };

    inner_type
}

/// Extracts the inner type `T` from a `Vec<T>`.
fn extract_vec_inner_type(ty: &Type) -> &Type {
    let Type::Path(TypePath { path, .. }) = ty else {
        abort!(ty, "Expected a Vec<T> type.");
    };
    let Some(segment) = path.segments.last() else {
        abort!(path, "Invalid type path.");
    };
    if segment.ident != "Vec" {
        abort!(segment, "Expected a Vec for batch task.");
    }
    let PathArguments::AngleBracketed(args) = &segment.arguments else {
        abort!(segment, "Vec must have a generic argument.");
    };
    let Some(GenericArgument::Type(inner)) = args.args.first() else {
        abort!(args, "Vec must have a type argument.");
    };
    inner
}

/// Extracts the item type `T` from a type that is `impl Iterator<Item = T>`.
fn extract_iterator_item_type(ty: &Type) -> &Type {
    if let Type::ImplTrait(impl_trait) = ty {
        for bound in &impl_trait.bounds {
            if let syn::TypeParamBound::Trait(trait_bound) = bound {
                if let Some(segment) = trait_bound.path.segments.last() {
                    if segment.ident == "Iterator" {
                        if let PathArguments::AngleBracketed(args) = &segment.arguments {
                            for arg in &args.args {
                                if let GenericArgument::AssocType(assoc) = arg {
                                    if assoc.ident == "Item" {
                                        return &assoc.ty;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    abort!(
        ty,
        "Stream task must return `impl Iterator<Item = Result<...>>`"
    );
}

// --- Main Macro Logic ---

pub fn task(attr: TokenStream, item: TokenStream) -> TokenStream {
    // 1. Parse attributes and the function definition
    let args = parse_macro_input!(attr as TaskArgs);
    let func = parse_macro_input!(item as ItemFn);

    // 2. Prepare identifiers and preserve the original function
    let func_name = &func.sig.ident;
    let struct_name = format_ident!("__{}", func_name);
    let task_impl_func_name = format_ident!("__impl_{}", func_name);
    let task_impl_func = {
        let mut f = func.clone();
        f.sig.ident = task_impl_func_name.clone();
        quote! { #f }
    };

    // 3. Analyze function signature (arguments and return type)
    let (arg_names, arg_types): (Vec<_>, Vec<_>) = func
        .sig
        .inputs
        .iter()
        .map(|arg| {
            let syn::FnArg::Typed(pt) = arg else {
                abort!(arg, "`self` arguments are not supported in tasks");
            };
            let Pat::Ident(pat_ident) = &*pt.pat else {
                abort!(
                    pt,
                    "Only simple identifiers are supported for task arguments"
                );
            };
            (pat_ident.ident.clone(), pt.ty.clone())
        })
        .unzip();

    let ReturnType::Type(_, return_ty) = &func.sig.output else {
        abort!(func.sig.output, "Task function must have a return type");
    };

    // 4. Generate the task struct and trait implementations
    let task_trait_impl = match args.task_type {
        TaskType::Single => {
            let output_type = extract_result_type(return_ty);
            let input_type = if arg_types.len() == 1 {
                let ty = &arg_types[0];
                quote! { #ty }
            } else if arg_types.is_empty() {
                quote! { () }
            } else {
                quote! { (#(#arg_types),*) }
            };

            let input_destructuring = if arg_names.len() > 1 {
                quote! { let (#(#arg_names),*) = input; }
            } else if !arg_names.is_empty() {
                let name = &arg_names[0];
                quote! { let #name = input; }
            } else {
                quote! { let () = input; }
            };

            let call_args = quote! { #(#arg_names),* };

            quote! {
                #[allow(non_camel_case_types)]
                struct #struct_name;

                impl<Id, C> task::Task<Id, C> for #struct_name
                where
                    Id: Clone,
                    C: task::TaskContext<Id>,
                {
                    fn prepare(&mut self) -> anyhow::Result<()> { Ok(()) }

                    fn run(&mut self, context: C) -> anyhow::Result<()> {
                        task::SingleTask::run(self, context)
                    }
                }

                impl<Id, C> task::SingleTask<Id, C> for #struct_name
                where
                    Id: Clone,
                    C: task::TaskContext<Id>,
                {
                    type Input = #input_type;
                    type Output = #output_type;
                    fn call(&mut self, input: Self::Input) -> anyhow::Result<Self::Output> {
                        #input_destructuring
                        #task_impl_func_name(#call_args)
                    }
                }
            }
        }
        TaskType::Batch => {
            if arg_types.len() != 1 {
                abort!(
                    func.sig.inputs,
                    "Batch task must have exactly one argument: Vec<Input>"
                );
            }
            let batch_size = args.batch_size.unwrap_or(16);
            let input_vec_type = &arg_types[0];
            let input_type = extract_vec_inner_type(input_vec_type);

            // For `fn(Vec<A>) -> Vec<Result<B>>`, `output_type` should be `B`.
            // 1. Get `Vec<Result<B>>` from the function's return type.
            // 2. Get `Result<B>` from `Vec<Result<B>>` using `extract_vec_inner_type`.
            // 3. Get `B` from `Result<B>` using `extract_result_type`.
            let output_result_type = extract_vec_inner_type(return_ty);
            let output_type = extract_result_type(output_result_type);

            quote! {
                #[allow(non_camel_case_types)]
                struct #struct_name;

                impl<Id, C> task::Task<Id, C> for #struct_name
                where
                    Id: Clone,
                    C: task::TaskContext<Id>,
                {
                    fn prepare(&mut self) -> anyhow::Result<()> { Ok(()) }
                    fn run(&mut self, context: C) -> anyhow::Result<()> {
                        task::BatchedTask::run(self, context)
                    }
                }

                impl<Id, C> task::BatchedTask<Id, C> for #struct_name
                where
                    Id: Clone,
                    C: task::TaskContext<Id>,
                {
                    type Input = #input_type;
                    type Output = #output_type;

                    fn batch_size(&self) -> usize { #batch_size }

                    fn call(&mut self, input: Vec<Self::Input>) -> Vec<anyhow::Result<Self::Output>> {
                        #task_impl_func_name(input)
                    }
                }
            }
        }
        TaskType::Stream => {
            if arg_types.len() != 1 {
                abort!(
                    func.sig.inputs,
                    "Stream task must have exactly one argument: Input"
                );
            }
            let input_type = &arg_types[0];
            let output_iterator_type = extract_result_type(return_ty); // `impl Iterator<...>`
            let output_type = extract_result_type(extract_iterator_item_type(output_iterator_type)); // `T` from `Item=Result<T>`

            quote! {
                #[allow(non_camel_case_types)]
                struct #struct_name;

                impl<Id, C> task::Task<Id, C> for #struct_name
                where
                    Id: Clone,
                    C: task::TaskContext<Id>,
                {
                    fn prepare(&mut self) -> anyhow::Result<()> { Ok(()) }
                    fn run(&mut self, context: C) -> anyhow::Result<()> {
                        task::StreamTask::run(self, context)
                    }
                }

                impl<Id, C> task::StreamTask<Id, C> for #struct_name
                where
                    Id: Clone,
                    C: task::TaskContext<Id>,
                {
                    type Input = #input_type;
                    type Output = #output_type;
                    fn call(&mut self, input: Self::Input) -> impl Iterator<Item = anyhow::Result<Self::Output>> {
                        #task_impl_func_name(input)
                    }
                }
            }
        }
    };

    // 5. Generate the graph constructor function (for the `#[workflow]` macro)
    let output_type = match args.task_type {
        TaskType::Single => extract_result_type(return_ty).clone(),
        TaskType::Batch => {
            // For `fn(Vec<A>) -> Vec<Result<B>>`, the graph output type is `Vec<B>`.
            let output_result_type = extract_vec_inner_type(return_ty);
            let inner_type = extract_result_type(output_result_type);
            parse_quote!(Vec<#inner_type>)
        }
        TaskType::Stream => {
            let output_iterator_type = extract_result_type(return_ty);
            let inner_type = extract_result_type(extract_iterator_item_type(output_iterator_type));
            parse_quote!(Vec<#inner_type>)
        }
    };

    let mut constructor_sig = func.sig.clone();
    constructor_sig.generics = parse_quote! { <G: 'static> };
    constructor_sig.inputs.clear();
    constructor_sig
        .inputs
        .push(parse_quote! { builder: &graph::Builder<G> });
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
    let task_id_str = format!("{}::{}", func.sig.ident, file!());
    let registrar_name = format_ident!("__factory_{}", func_name);
    let static_registrar_name = format_ident!("__REG_ONCE_{}", func_name);

    let factory_logic = match args.task_type {
        TaskType::Single => {
            let call_args = quote! { #(#arg_names),* };
            quote! {
                std::sync::Arc::new(|__inputs| {
                    #(#value_downcasts)*
                    // This is for local execution, which doesn't use the new Task traits yet.
                    // We directly call the user's function logic.
                    let result = #task_impl_func_name(#call_args).unwrap();
                    std::sync::Arc::new(result) as graph::Value
                })
            }
        }
        TaskType::Batch => {
            let arg_name = &arg_names[0];
            let arg_ty = &arg_types[0];
            quote! {
                std::sync::Arc::new(|__inputs| {
                    let #arg_name = __inputs[0].downcast_ref::<#arg_ty>().unwrap().clone();
                    let result = #task_impl_func_name(#arg_name).unwrap();
                    std::sync::Arc::new(result) as graph::Value
                })
            }
        }
        TaskType::Stream => {
            let arg_name = &arg_names[0];
            let arg_ty = &arg_types[0];
            quote! {
                std::sync::Arc::new(|__inputs| {
                    let #arg_name = __inputs[0].downcast_ref::<#arg_ty>().unwrap().clone();
                    let result_iter = #task_impl_func_name(#arg_name).unwrap();
                    let result_vec: Vec<_> = result_iter.map(|item| item.unwrap()).collect();
                    std::sync::Arc::new(result_vec) as graph::Value
                })
            }
        }
    };

    let constructor = quote! {
        // This factory creates the executable closure for the graph's local `run()` method.
        fn #registrar_name() -> graph::TaskFactory {
            std::sync::Arc::new(|| {
                #factory_logic
            })
        }

        // This is the function called inside a `#[workflow]`.
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

    // 6. Combine all generated code into the final TokenStream
    quote! {
        #task_impl_func
        #task_trait_impl
        #constructor
    }
    .into()
}
