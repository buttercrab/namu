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
use proc_macro2::TokenStream as TokenStream2;
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

// --- Generator Functions ---

struct TaskDefinition<'a> {
    func_name: &'a Ident,
    struct_name: &'a Ident,
    impl_func_name: &'a Ident,
    args: &'a TaskArgs,
    arg_names: &'a [Ident],
    arg_types: &'a [Box<Type>],
    return_ty: &'a Type,
}

fn generate_single_task_impl(def: &TaskDefinition) -> TokenStream2 {
    let arg_types = def.arg_types;
    let arg_names = def.arg_names;
    let struct_name = def.struct_name;
    let impl_func_name = def.impl_func_name;

    let output_type = extract_result_type(def.return_ty);
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
                #impl_func_name(#call_args)
            }
        }
    }
}

fn generate_batch_task_impl(def: &TaskDefinition) -> TokenStream2 {
    let struct_name = def.struct_name;
    let impl_func_name = def.impl_func_name;

    if def.arg_types.len() != 1 {
        abort!(
            def.func_name.span(),
            "Batch task must have exactly one argument: Vec<Input>"
        );
    }
    let batch_size = def.args.batch_size.unwrap_or(16);
    let input_vec_type = &def.arg_types[0];
    let input_type = extract_vec_inner_type(input_vec_type);

    let output_result_type = extract_vec_inner_type(def.return_ty);
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
                #impl_func_name(input)
            }
        }
    }
}

fn generate_stream_task_impl(def: &TaskDefinition) -> TokenStream2 {
    let arg_types = def.arg_types;
    let arg_names = def.arg_names;
    let struct_name = def.struct_name;
    let impl_func_name = def.impl_func_name;

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

    let output_iterator_type = extract_result_type(def.return_ty);
    let output_type = extract_result_type(extract_iterator_item_type(output_iterator_type));

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
                #input_destructuring
                #impl_func_name(#call_args).unwrap()
            }
        }
    }
}

fn generate_constructor(def: &TaskDefinition, original_sig: &syn::Signature) -> TokenStream2 {
    let func_name = def.func_name;
    let impl_func_name = def.impl_func_name;
    let arg_names = def.arg_names;
    let arg_types = def.arg_types;

    let output_type = match def.args.task_type {
        TaskType::Single => extract_result_type(def.return_ty).clone(),
        TaskType::Batch => {
            let output_result_type = extract_vec_inner_type(def.return_ty);
            extract_result_type(output_result_type).clone()
        }
        TaskType::Stream => {
            let output_iterator_type = extract_result_type(def.return_ty);
            extract_result_type(extract_iterator_item_type(output_iterator_type)).clone()
        }
    };

    let mut constructor_sig = original_sig.clone();
    constructor_sig.generics = parse_quote! { <G: 'static> };
    constructor_sig.inputs.clear();
    constructor_sig
        .inputs
        .push(parse_quote! { builder: &graph::Builder<G> });

    match def.args.task_type {
        TaskType::Single | TaskType::Stream => {
            for (name, ty) in arg_names.iter().zip(arg_types.iter()) {
                constructor_sig
                    .inputs
                    .push(parse_quote! { #name: graph::TracedValue<#ty> });
            }
        }
        TaskType::Batch => {
            let name = &arg_names[0];
            let vec_ty = &arg_types[0];
            let inner_ty = extract_vec_inner_type(vec_ty);
            constructor_sig
                .inputs
                .push(parse_quote! { #name: graph::TracedValue<#inner_ty> });
        }
    }

    constructor_sig.output = parse_quote! { -> graph::TracedValue<#output_type> };

    let value_downcasts = arg_names.iter().enumerate().map(|(i, name)| {
        let ty = &arg_types[i];
        quote! { let #name = __inputs[#i].downcast_ref::<#ty>().unwrap().clone(); }
    });
    let input_ids = arg_names.iter().map(|name| quote! { #name.id });
    let task_id_str = format!("{}::{}", original_sig.ident, file!());
    let factory_func_name = format_ident!("__factory_{}", func_name);
    let static_registrar_name = format_ident!("__REG_ONCE_{}", func_name);

    let factory_logic = match def.args.task_type {
        TaskType::Single => {
            let call_args = quote! { #(#arg_names),* };
            quote! {
                std::sync::Arc::new(|__inputs| {
                    #(#value_downcasts)*
                    let result = #impl_func_name(#call_args).unwrap();
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
                    let result = #impl_func_name(#arg_name).unwrap();
                    std::sync::Arc::new(result) as graph::Value
                })
            }
        }
        TaskType::Stream => {
            let call_args = quote! { #(#arg_names),* };
            quote! {
                std::sync::Arc::new(|__inputs| {
                    #(#value_downcasts)*
                    let result_iter = #impl_func_name(#call_args).unwrap();
                    let result_vec: Vec<_> = result_iter.map(|item| item.unwrap()).collect();
                    std::sync::Arc::new(result_vec) as graph::Value
                })
            }
        }
    };

    quote! {
        fn #factory_func_name() -> graph::TaskFactory {
            std::sync::Arc::new(|| {
                #factory_logic
            })
        }

        #[allow(non_snake_case)]
        pub #constructor_sig {
            #[allow(non_upper_case_globals)]
            static #static_registrar_name: std::sync::Once = std::sync::Once::new();
            #static_registrar_name.call_once(|| {
                graph::register_task(#task_id_str.to_string(), #factory_func_name());
            });

            let kind = graph::NodeKind::Call {
                name: stringify!(#func_name),
                task_id: #task_id_str.to_string(),
                inputs: vec![#(#input_ids),*],
            };
            builder.add_instruction(kind)
        }
    }
}

// --- Main Macro Logic ---

pub fn task(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as TaskArgs);
    let func = parse_macro_input!(item as ItemFn);

    let func_name = &func.sig.ident;
    let struct_name = format_ident!("__{}", func_name);
    let impl_func_name = format_ident!("__impl_{}", func_name);

    let impl_func = {
        let mut f = func.clone();
        f.sig.ident = impl_func_name.clone();
        quote! { #f }
    };

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

    let def = TaskDefinition {
        func_name,
        struct_name: &struct_name,
        impl_func_name: &impl_func_name,
        args: &args,
        arg_names: &arg_names,
        arg_types: &arg_types,
        return_ty,
    };

    let task_trait_impl = match args.task_type {
        TaskType::Single => generate_single_task_impl(&def),
        TaskType::Batch => generate_batch_task_impl(&def),
        TaskType::Stream => generate_stream_task_impl(&def),
    };

    let constructor = generate_constructor(&def, &func.sig);

    quote! {
        #impl_func
        #task_trait_impl
        #constructor
    }
    .into()
}
