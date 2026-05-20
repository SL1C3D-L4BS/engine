//! `engine-ecs-macro` — the workspace's derive macros.
//!
//! Level 0 crate. See `ENGINE_SPECIFICATION_v2.0.md` Part IV.1 and ADR-024.
//!
//! Per ADR-024 this single proc-macro crate hosts *all* of the engine's derive
//! macros, not only ECS ones — Rust requires derive macros to live in a
//! dedicated `proc-macro` crate, and the spec's Level 0 crate list names only
//! one such crate.
//!
//! - [`macro@Component`] — implements `engine_core::ecs::Component`.
//! - [`macro@Reflect`] — implements `engine_reflect::Reflect`.
//!
//! The generated code refers to `::engine_core` and `::engine_reflect` by
//! their canonical names; a consumer must depend on those crates unrenamed.

use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, Ident, parse_macro_input};

/// Derives `engine_core::ecs::Component`.
///
/// Storage defaults to the archetype `Table`. Opt into sparse storage with
/// `#[component(storage = "SparseSet")]` (spec IV.3 / ADR-002).
#[proc_macro_derive(Component, attributes(component))]
pub fn derive_component(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let storage = match parse_storage(&input) {
        Ok(s) => s,
        Err(e) => return e.to_compile_error().into(),
    };

    quote! {
        impl #impl_generics ::engine_core::ecs::Component for #name #ty_generics #where_clause {
            const STORAGE: ::engine_core::ecs::StorageKind =
                ::engine_core::ecs::StorageKind::#storage;
        }
    }
    .into()
}

/// Reads the `storage` value out of `#[component(...)]`, defaulting to `Table`.
fn parse_storage(input: &DeriveInput) -> syn::Result<Ident> {
    let mut storage = Ident::new("Table", proc_macro2::Span::call_site());
    for attr in &input.attrs {
        if !attr.path().is_ident("component") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("storage") {
                let value: syn::LitStr = meta.value()?.parse()?;
                match value.value().as_str() {
                    "Table" => storage = Ident::new("Table", value.span()),
                    "SparseSet" => storage = Ident::new("SparseSet", value.span()),
                    other => {
                        return Err(syn::Error::new_spanned(
                            &value,
                            format!("unknown storage `{other}`; expected `Table` or `SparseSet`"),
                        ));
                    }
                }
                Ok(())
            } else {
                Err(meta.error("unknown `component` attribute key; expected `storage`"))
            }
        })?;
    }
    Ok(storage)
}

/// Derives `engine_reflect::Reflect` for a struct with named fields.
///
/// Every field type must implement `engine_reflect::FromReflect` and convert
/// into `engine_reflect::ReflectValue` (the primitive types do).
#[proc_macro_derive(Reflect)]
pub fn derive_reflect(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;
    let name_str = name.to_string();
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let fields = match &input.data {
        Data::Struct(s) => match &s.fields {
            Fields::Named(named) => &named.named,
            _ => {
                return syn::Error::new_spanned(
                    name,
                    "`Reflect` can only be derived for structs with named fields",
                )
                .to_compile_error()
                .into();
            }
        },
        _ => {
            return syn::Error::new_spanned(name, "`Reflect` can only be derived for structs")
                .to_compile_error()
                .into();
        }
    };

    let idents: Vec<&Ident> = fields.iter().map(|f| f.ident.as_ref().unwrap()).collect();
    let types: Vec<&syn::Type> = fields.iter().map(|f| &f.ty).collect();
    let names: Vec<String> = idents.iter().map(|i| i.to_string()).collect();
    let indices: Vec<usize> = (0..idents.len()).collect();
    let count = idents.len();

    quote! {
        impl #impl_generics ::engine_reflect::Reflect for #name #ty_generics #where_clause {
            fn type_name(&self) -> &'static str {
                #name_str
            }

            fn field_count(&self) -> usize {
                #count
            }

            fn field_name(&self, index: usize) -> ::core::option::Option<&'static str> {
                match index {
                    #( #indices => ::core::option::Option::Some(#names), )*
                    _ => ::core::option::Option::None,
                }
            }

            fn get_field(
                &self,
                name: &str,
            ) -> ::core::option::Option<::engine_reflect::ReflectValue> {
                match name {
                    #( #names => ::core::option::Option::Some(
                        ::engine_reflect::ReflectValue::from(
                            ::core::clone::Clone::clone(&self.#idents),
                        )
                    ), )*
                    _ => ::core::option::Option::None,
                }
            }

            fn set_field(
                &mut self,
                name: &str,
                value: ::engine_reflect::ReflectValue,
            ) -> bool {
                match name {
                    #( #names => {
                        match <#types as ::engine_reflect::FromReflect>::from_reflect(value) {
                            ::core::option::Option::Some(v) => {
                                self.#idents = v;
                                true
                            }
                            ::core::option::Option::None => false,
                        }
                    } )*
                    _ => false,
                }
            }
        }
    }
    .into()
}
