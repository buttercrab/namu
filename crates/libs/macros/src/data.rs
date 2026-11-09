//! `#[derive(Data)]`: a type the engine holds as an opaque value (design §8.2), and describes in
//! the IR by its shape: a struct's fields, an enum's variants (design §5). Every type parameter
//! must be `Data` too, and so must every field.
//!
//! `#[derive(Describe)]` describes a type without making it `Data`: for a task's types, which only
//! need to say what they are (`namu_guest::Describe`).
//!
//! A shape is what crosses, so serde's attributes count: fields and variants are named as serde
//! names them (`rename`, `rename_all`), and skipped ones are left out. A type using an attribute
//! that changes its shape in a way not modelled here (`flatten`, `tag`, `untagged`, `default`,
//! `with`, ...) is described by its name alone, with its shape unknown, so it is not checked
//! rather than checked wrongly.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Attribute, Data as Body, DeriveInput, Fields, Token, parse_macro_input, parse_quote};

/// `#[derive(Data)]`: `Describe` and `Data`.
pub fn derive_data(input: TokenStream) -> TokenStream {
    let mut input = parse_macro_input!(input as DeriveInput);
    let exports = quote! { ::namu::__macro_exports };
    let describe = describe(&input, &exports, quote! { ::namu::Data });
    for param in input.generics.type_params_mut() {
        param.bounds.push(parse_quote!(::namu::Data));
    }
    let name = &input.ident;
    let (impl_generics, type_generics, where_clause) = input.generics.split_for_impl();
    quote! {
        #describe
        impl #impl_generics ::namu::Data for #name #type_generics #where_clause {}
    }
    .into()
}

/// `#[derive(Describe)]`, with the types it names under `exports`: `namu`'s or `namu_guest`'s.
pub fn derive_describe(input: TokenStream, exports: TokenStream2) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let bound = quote! { #exports::Describe };
    describe(&input, &exports, bound).into()
}

fn describe(input: &DeriveInput, exports: &TokenStream2, bound: TokenStream2) -> TokenStream2 {
    let mut generics = input.generics.clone();
    for param in generics.type_params_mut() {
        param.bounds.push(parse_quote!(#bound));
    }
    let name = &input.ident;
    let (impl_generics, type_generics, where_clause) = generics.split_for_impl();
    let shape = match shape(input, exports) {
        Ok(shape) => shape,
        Err(err) => return err.to_compile_error(),
    };
    let body = match shape {
        Some(def) => quote! { #exports::named(::std::any::type_name::<Self>(), defs, |defs| #def) },
        None => quote! { #exports::Type::Named(::std::any::type_name::<Self>().to_string()) },
    };
    let names = match serde_names(input) {
        Ok(names) => names,
        Err(err) => return err.to_compile_error(),
    };
    quote! {
        impl #impl_generics #exports::Describe for #name #type_generics #where_clause {
            fn describe(defs: &mut #exports::Defs) -> #exports::Type {
                let _ = &defs;
                #body
            }

            #names
        }
    }
}

/// The type's `Def`, or `None` if its shape is not modelled.
fn shape(input: &DeriveInput, exports: &TokenStream2) -> syn::Result<Option<TokenStream2>> {
    let container = Serde::of(&input.attrs)?;
    if container.unmodelled || container.may_be_missing {
        return Ok(None);
    }
    Ok(match &input.data {
        Body::Struct(body) => fields(&body.fields, container.rename_all.as_deref(), exports)?
            .map(|fields| quote! { #exports::Def::Struct(#fields) }),
        Body::Enum(body) => {
            let mut variants = Vec::new();
            for variant in &body.variants {
                let serde = Serde::of(&variant.attrs)?;
                if serde.unmodelled {
                    return Ok(None);
                }
                if serde.skip {
                    continue;
                }
                let name = serde.rename.unwrap_or_else(|| {
                    rename(
                        &variant.ident.to_string(),
                        container.rename_all.as_deref(),
                        Case::Variant,
                    )
                });
                let Some(fields) = fields(&variant.fields, serde.rename_all.as_deref(), exports)?
                else {
                    return Ok(None);
                };
                variants.push(quote! { (#name.to_string(), #fields) });
            }
            Some(quote! { #exports::Def::Enum(vec![#(#variants),*]) })
        }
        Body::Union(body) => {
            return Err(syn::Error::new_spanned(
                body.union_token,
                "a union cannot be described",
            ));
        }
    })
}

/// An enum's `serde_name`: its variants' and their fields' names in Rust, to serde's. Nothing for
/// a struct, or an enum serde writes as Rust does.
fn serde_names(input: &DeriveInput) -> syn::Result<TokenStream2> {
    let Body::Enum(body) = &input.data else {
        return Ok(TokenStream2::new());
    };
    let container = Serde::of(&input.attrs)?;
    let mut arms = Vec::new();
    for variant in &body.variants {
        let serde = Serde::of(&variant.attrs)?;
        let rust = variant.ident.to_string();
        let name = serde
            .rename
            .unwrap_or_else(|| rename(&rust, container.rename_all.as_deref(), Case::Variant));
        if name != rust {
            arms.push(quote! { (#rust, None) => Some(#name), });
        }
        for field in &variant.fields {
            let Some(ident) = &field.ident else {
                continue;
            };
            let field_serde = Serde::of(&field.attrs)?;
            let field_rust = ident.to_string();
            let field_name = field_serde
                .rename
                .unwrap_or_else(|| rename(&field_rust, serde.rename_all.as_deref(), Case::Field));
            if field_name != field_rust {
                arms.push(quote! { (#rust, Some(#field_rust)) => Some(#field_name), });
            }
        }
    }
    if arms.is_empty() {
        return Ok(TokenStream2::new());
    }
    Ok(quote! {
        fn serde_name(
            variant: &str,
            field: ::std::option::Option<&str>,
        ) -> ::std::option::Option<&'static str> {
            match (variant, field) {
                #(#arms)*
                _ => None,
            }
        }
    })
}

/// The fields as `(name, type)` pairs, named as serde names them; a tuple struct's are named by
/// position. `None` if their shape is not modelled.
fn fields(
    fields: &Fields,
    rename_all: Option<&str>,
    exports: &TokenStream2,
) -> syn::Result<Option<TokenStream2>> {
    let mut described = Vec::new();
    for (index, field) in fields.iter().enumerate() {
        let serde = Serde::of(&field.attrs)?;
        let optional = is_option(&field.ty);
        if serde.unmodelled || (serde.may_be_missing && !optional) {
            return Ok(None);
        }
        if serde.skip {
            continue;
        }
        let name = match (&field.ident, serde.rename) {
            (_, Some(name)) => name,
            (Some(ident), None) => rename(&ident.to_string(), rename_all, Case::Field),
            (None, None) => index.to_string(),
        };
        let ty = &field.ty;
        described.push(quote! { (#name.to_string(), <#ty as #exports::Describe>::describe(defs)) });
    }
    Ok(Some(quote! { vec![#(#described),*] }))
}

/// What `#[serde(...)]` says of a container, variant or field, as far as its shape goes.
#[derive(Default)]
struct Serde {
    rename: Option<String>,
    rename_all: Option<String>,
    /// Left out both ways.
    skip: bool,
    /// May be missing when written or read (`default`, `skip_serializing_if`): fine for an
    /// `Option`, which is read as none when missing anyway.
    may_be_missing: bool,
    /// Changes the shape in a way not modelled.
    unmodelled: bool,
}

impl Serde {
    fn of(attrs: &[Attribute]) -> syn::Result<Self> {
        let mut serde = Self::default();
        for attr in attrs.iter().filter(|attr| attr.path().is_ident("serde")) {
            attr.parse_nested_meta(|meta| {
                let key = meta
                    .path
                    .get_ident()
                    .map(ToString::to_string)
                    .unwrap_or_default();
                let value = if meta.input.peek(Token![=]) {
                    Some(meta.value()?.parse::<syn::Expr>()?)
                } else if meta.input.peek(syn::token::Paren) {
                    // `rename(serialize = .., deserialize = ..)` and the like: a name or rule
                    // for one direction only.
                    let content;
                    syn::parenthesized!(content in meta.input);
                    content.parse::<TokenStream2>()?;
                    serde.unmodelled |= matches!(key.as_str(), "rename" | "rename_all");
                    return Ok(());
                } else {
                    None
                };
                let text = || match &value {
                    Some(syn::Expr::Lit(syn::ExprLit {
                        lit: syn::Lit::Str(text),
                        ..
                    })) => Ok(text.value()),
                    _ => Err(meta.error("expected a string")),
                };
                match key.as_str() {
                    "rename" => serde.rename = Some(text()?),
                    "rename_all" => serde.rename_all = Some(text()?),
                    "skip" => serde.skip = true,
                    "default" | "skip_serializing_if" => serde.may_be_missing = true,
                    // Names and checks that do not change what crosses.
                    "alias"
                    | "bound"
                    | "crate"
                    | "deny_unknown_fields"
                    | "expecting"
                    | "borrow"
                    | "getter"
                    | "variant_identifier"
                    | "field_identifier" => {}
                    _ => serde.unmodelled = true,
                }
                Ok(())
            })?;
        }
        Ok(serde)
    }
}

/// `Option<..>`, as written.
fn is_option(ty: &syn::Type) -> bool {
    matches!(ty, syn::Type::Path(path)
        if path.qself.is_none()
            && path.path.segments.last().is_some_and(|segment| segment.ident == "Option"))
}

/// What a name is in Rust, for serde's `rename_all`: fields are `snake_case`, variants
/// `PascalCase`.
#[derive(Clone, Copy)]
enum Case {
    Field,
    Variant,
}

/// `name` under serde's `rename_all` rule, if any.
fn rename(name: &str, rule: Option<&str>, case: Case) -> String {
    let name = name.strip_prefix("r#").unwrap_or(name);
    let Some(rule) = rule else {
        return name.to_string();
    };
    // The name's words, lowercase.
    let words: Vec<String> = match case {
        Case::Field => name.split('_').map(str::to_lowercase).collect(),
        Case::Variant => {
            let mut words: Vec<String> = Vec::new();
            for c in name.chars() {
                match words.last_mut() {
                    Some(word) if !c.is_uppercase() => word.push(c),
                    _ => words.push(c.to_lowercase().collect()),
                }
            }
            words
        }
    };
    let capitalized = |word: &String| {
        let mut chars = word.chars();
        chars.next().map_or_else(String::new, |first| {
            first.to_uppercase().chain(chars).collect()
        })
    };
    match rule {
        "lowercase" => words.concat(),
        "UPPERCASE" => words.concat().to_uppercase(),
        "PascalCase" => words.iter().map(capitalized).collect(),
        "camelCase" => {
            let mut out = words.first().cloned().unwrap_or_default();
            out.extend(words.iter().skip(1).map(capitalized));
            out
        }
        "snake_case" => words.join("_"),
        "SCREAMING_SNAKE_CASE" => words.join("_").to_uppercase(),
        "kebab-case" => words.join("-"),
        "SCREAMING-KEBAB-CASE" => words.join("-").to_uppercase(),
        _ => name.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_serde_rename_all() {
        let field = |rule| rename("page_url", Some(rule), Case::Field);
        let variant = |rule| rename("NotFound", Some(rule), Case::Variant);
        for (rule, as_field, as_variant) in [
            ("lowercase", "pageurl", "notfound"),
            ("UPPERCASE", "PAGEURL", "NOTFOUND"),
            ("PascalCase", "PageUrl", "NotFound"),
            ("camelCase", "pageUrl", "notFound"),
            ("snake_case", "page_url", "not_found"),
            ("SCREAMING_SNAKE_CASE", "PAGE_URL", "NOT_FOUND"),
            ("kebab-case", "page-url", "not-found"),
            ("SCREAMING-KEBAB-CASE", "PAGE-URL", "NOT-FOUND"),
        ] {
            assert_eq!(field(rule), as_field, "{rule}");
            assert_eq!(variant(rule), as_variant, "{rule}");
        }
        assert_eq!(rename("r#type", None, Case::Field), "type");
    }
}
