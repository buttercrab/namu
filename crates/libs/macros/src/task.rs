//! The `#[task]` attribute: declares a task that a workflow can call.
//!
//! A task is declared by its signature only. Its implementation lives elsewhere (a WASM component,
//! a native library, or an in-process function registered with the engine), so a workflow is
//! type-checked against the signature without linking any task code:
//!
//! ```ignore
//! #[task]
//! pub fn analyze(doc: String, cfg: Config) -> Result<Stats>;
//!
//! #[task(batch)]
//! pub fn embed(texts: Vec<String>) -> Vec<Result<Embedding>>;
//!
//! #[task(stream)]
//! pub fn crawl(seed: Url) -> Result<impl Iterator<Item = Result<Page>>>;
//! ```
//!
//! The macro replaces the signature with a graph constructor of the same name. Inside a
//! `#[workflow]` it adds a `Call` node, and each argument goes through `IntoArg`, which is where
//! fan-out and collect are resolved from the parameter types.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::{
    GenericArgument, Ident, Pat, PathArguments, ReturnType, Token, Type, TypePath,
    parse_macro_input,
};

/// Returns a compile error spanning `tokens` from the enclosing function.
macro_rules! bail {
    ($tokens:expr, $msg:expr $(,)?) => {
        return Err(syn::Error::new_spanned(&$tokens, $msg))
    };
}

/// How a task is called. It changes only the types the constructor accepts and returns.
#[derive(Debug, Clone, Copy, Default)]
enum TaskKind {
    /// One input, one result.
    #[default]
    Single,
    /// Written for a list of inputs, called per item; the engine groups the items.
    Batch,
    /// Yields its results over time.
    Stream,
}

impl Parse for TaskKind {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        if input.is_empty() {
            return Ok(Self::default());
        }
        let ident: Ident = input.parse()?;
        let kind = match ident.to_string().as_str() {
            "single" => Self::Single,
            "batch" => Self::Batch,
            "stream" => Self::Stream,
            _ => bail!(ident, "expected `single`, `batch`, or `stream`"),
        };
        if !input.is_empty() {
            return Err(input.error("unexpected task argument"));
        }
        Ok(kind)
    }
}

/// `pub fn name(args...) -> Ret;`
struct TaskSignature {
    vis: syn::Visibility,
    sig: syn::Signature,
}

impl Parse for TaskSignature {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        input.call(syn::Attribute::parse_outer)?;
        let vis = input.parse()?;
        let sig: syn::Signature = input.parse()?;
        if input.peek(syn::token::Brace) {
            return Err(input.error(
                "a task is declared by its signature only (end it with `;`); \
                 its implementation is registered with the engine or built with `namu_guest`",
            ));
        }
        input.parse::<Token![;]>()?;
        Ok(Self { vis, sig })
    }
}

pub fn task(attr: TokenStream, item: TokenStream) -> TokenStream {
    let kind = parse_macro_input!(attr as TaskKind);
    let signature = parse_macro_input!(item as TaskSignature);
    constructor(kind, &signature.vis, &signature.sig)
        .unwrap_or_else(|err| err.to_compile_error())
        .into()
}

fn constructor(
    kind: TaskKind,
    vis: &syn::Visibility,
    sig: &syn::Signature,
) -> syn::Result<TokenStream2> {
    let name = &sig.ident;
    let mut params = Vec::new();
    for arg in &sig.inputs {
        let syn::FnArg::Typed(pt) = arg else {
            bail!(arg, "tasks cannot take `self`");
        };
        let Pat::Ident(pat) = &*pt.pat else {
            bail!(pt, "task arguments must be plain identifiers");
        };
        params.push((pat.ident.clone(), (*pt.ty).clone()));
    }
    let ReturnType::Type(_, ret) = &sig.output else {
        bail!(sig.output, "a task must return a `Result`");
    };

    // What one call yields, per item for batch and stream tasks.
    let output = match kind {
        TaskKind::Single => result_inner(ret)?,
        TaskKind::Batch => {
            let [(_, input)] = params.as_mut_slice() else {
                bail!(
                    sig.inputs,
                    "a batch task takes exactly one `Vec<T>` argument"
                );
            };
            *input = generic_arg(input, "Vec", "a batch task takes a `Vec<T>`")?.clone();
            result_inner(generic_arg(
                ret,
                "Vec",
                "a batch task returns `Vec<Result<T>>`",
            )?)?
        }
        TaskKind::Stream => result_inner(iterator_item(result_inner(ret)?)?)?,
    };

    let arg_names = params.iter().map(|(name, _)| name);
    let arg_decls = params
        .iter()
        .map(|(name, ty)| quote! { #name: impl ::namu::__macro_exports::IntoArg<#ty> });

    // A non-empty tuple is several outputs; anything else (including `()`) is one.
    let (ret_ty, call_fn) = match output {
        Type::Tuple(tuple) if !tuple.elems.is_empty() => {
            let elems = tuple.elems.iter();
            (
                quote! { ( #( ::namu::__macro_exports::TracedValue<#elems> ),* ) },
                format_ident!("call{}", tuple.elems.len()),
            )
        }
        ty => (
            quote! { ::namu::__macro_exports::TracedValue<#ty> },
            format_ident!("call"),
        ),
    };

    Ok(quote! {
        #vis fn #name<G: 'static>(
            __builder: &::namu::__macro_exports::Builder<G>,
            #( #arg_decls ),*
        ) -> #ret_ty {
            // Resolving an argument may emit a fan-out or collect, so arguments are resolved in
            // order before the call.
            let __inputs = ::std::vec::Vec::from([
                #( ::namu::__macro_exports::IntoArg::into_arg(#arg_names, __builder) ),*
            ]);
            ::namu::__macro_exports::#call_fn(__builder, stringify!(#name), __inputs)
        }
    })
}

/// `T` from `Result<T>` or `Result<T, E>`.
fn result_inner(ty: &Type) -> syn::Result<&Type> {
    generic_arg(ty, "Result", "a task must return a `Result`")
}

/// The first type argument of `ty` if its last path segment is `outer`.
fn generic_arg<'a>(ty: &'a Type, outer: &str, msg: &str) -> syn::Result<&'a Type> {
    if let Type::Path(TypePath {
        qself: None, path, ..
    }) = ty
        && let Some(segment) = path.segments.last()
        && segment.ident == outer
        && let PathArguments::AngleBracketed(args) = &segment.arguments
        && let Some(GenericArgument::Type(inner)) = args.args.first()
    {
        return Ok(inner);
    }
    bail!(ty, msg)
}

/// `T` from `impl Iterator<Item = T>`.
fn iterator_item(ty: &Type) -> syn::Result<&Type> {
    if let Type::ImplTrait(impl_trait) = ty {
        for bound in &impl_trait.bounds {
            if let syn::TypeParamBound::Trait(trait_bound) = bound
                && let Some(segment) = trait_bound.path.segments.last()
                && segment.ident == "Iterator"
                && let PathArguments::AngleBracketed(args) = &segment.arguments
            {
                for arg in &args.args {
                    if let GenericArgument::AssocType(assoc) = arg
                        && assoc.ident == "Item"
                    {
                        return Ok(&assoc.ty);
                    }
                }
            }
        }
    }
    bail!(
        ty,
        "a stream task returns `Result<impl Iterator<Item = Result<T>>>`"
    )
}
