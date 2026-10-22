//! `#[derive(Data)]`: a type the engine holds as an opaque value (design §8.2), and describes in
//! the IR by its shape: a struct's fields, an enum's variants (design §5). Every type parameter
//! must be `Data` too, and so must every field.

use proc_macro::TokenStream;
use quote::quote;
use syn::{Data as Body, DeriveInput, Fields, parse_macro_input, parse_quote};

pub fn derive(input: TokenStream) -> TokenStream {
    let mut input = parse_macro_input!(input as DeriveInput);
    for param in input.generics.type_params_mut() {
        param.bounds.push(parse_quote!(::namu::Data));
    }
    let name = &input.ident;
    let (impl_generics, type_generics, where_clause) = input.generics.split_for_impl();
    let exports = quote! { ::namu::__macro_exports };
    let def = match &input.data {
        Body::Struct(body) => {
            let fields = fields(&body.fields);
            quote! { #exports::Def::Struct(#fields) }
        }
        Body::Enum(body) => {
            let variants = body.variants.iter().map(|variant| {
                let name = variant.ident.to_string();
                let fields = fields(&variant.fields);
                quote! { (#name.to_string(), #fields) }
            });
            quote! { #exports::Def::Enum(vec![#(#variants),*]) }
        }
        Body::Union(body) => {
            return syn::Error::new_spanned(body.union_token, "a union cannot be Data")
                .to_compile_error()
                .into();
        }
    };
    quote! {
        impl #impl_generics ::namu::Data for #name #type_generics #where_clause {
            fn describe(defs: &mut #exports::Defs) -> #exports::Type {
                #exports::named(::std::any::type_name::<Self>(), defs, |defs| #def)
            }
        }
    }
    .into()
}

/// The fields as `(name, type)` pairs; a tuple struct's are named by position.
fn fields(fields: &Fields) -> proc_macro2::TokenStream {
    let fields = fields.iter().enumerate().map(|(index, field)| {
        let name = field
            .ident
            .as_ref()
            .map_or_else(|| index.to_string(), ToString::to_string);
        let ty = &field.ty;
        quote! { (#name.to_string(), <#ty as ::namu::Data>::describe(defs)) }
    });
    quote! { vec![#(#fields),*] }
}
