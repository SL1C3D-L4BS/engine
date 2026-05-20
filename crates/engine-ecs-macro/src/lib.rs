//! `engine-ecs-macro` — the workspace's derive macros.
//!
//! Level 0 crate. See `ENGINE_SPECIFICATION_v2.0.md` Part IV.1 and ADR-024.
//!
//! Per ADR-024 this single proc-macro crate hosts *all* of the engine's derive
//! macros, not only ECS ones — Rust requires derive macros to live in a
//! dedicated `proc-macro` crate, and the spec's Level 0 crate list names only
//! one such crate.
//!
//! - [`macro@Component`] — implements `engine_core::ecs::Component`, including
//!   the [`TypeStableId`](engine_reflect::TypeStableId) used by the archetype
//!   index (ADR-031). The id is computed at macro-expansion time by hashing
//!   `crate_name || "::" || ident` with BLAKE3 and emitted as a literal `u64`,
//!   so the derived `STABLE_ID` is a `const` and no runtime hashing is needed.
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
///
/// Also emits the component's [`TypeStableId`](engine_reflect::TypeStableId)
/// as `const STABLE_ID` — a cross-architecture-stable 64-bit identifier the
/// archetype index uses in place of `std::any::TypeId` (ADR-031). The id is
/// derived from `BLAKE3(crate_name || "::" || ident)`, computed at
/// macro-expansion time, and emitted as a literal `u64`.
#[proc_macro_derive(Component, attributes(component))]
pub fn derive_component(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let storage = match parse_storage(&input) {
        Ok(s) => s,
        Err(e) => return e.to_compile_error().into(),
    };

    let stable_id_u64 = compute_stable_id(&name.to_string());

    quote! {
        impl #impl_generics ::engine_core::ecs::Component for #name #ty_generics #where_clause {
            const STORAGE: ::engine_core::ecs::StorageKind =
                ::engine_core::ecs::StorageKind::#storage;
            const STABLE_ID: ::engine_reflect::TypeStableId =
                ::engine_reflect::TypeStableId(#stable_id_u64);
        }
    }
    .into()
}

/// Computes the `TypeStableId` `u64` for a component named `ident`.
///
/// The hashed string is `crate_name || "::" || ident`, where `crate_name`
/// comes from the `CARGO_CRATE_NAME` environment variable cargo sets when it
/// invokes the proc-macro. The full Rust `module_path!()` is not visible at
/// expansion time, so this is the closest stable qualifier available — same
/// crate + same type ident is the practical uniqueness guarantee (ADR-031
/// "Risks and tradeoffs").
fn compute_stable_id(ident: &str) -> u64 {
    let crate_name = std::env::var("CARGO_CRATE_NAME").unwrap_or_default();
    let qualified = format!("{crate_name}::{ident}");
    let hash = blake3::hash(qualified.as_bytes());
    let bytes = hash.as_bytes();
    u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ])
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

/// Derives `engine_core::ecs::CanonicalBytes` for `Copy + 'static` components.
///
/// The implementation simply reinterprets `&Self` as `&[u8; size_of::<Self>()]`
/// and returns it. Used only by the Phase 3 replay-parity oracle (ADR-033) —
/// production code never calls it. Padding bytes contribute to the digest;
/// callers using this derive must use components with no implementation-
/// defined padding (typically `#[repr(C)]` plain-data structs).
#[proc_macro_derive(CanonicalBytes)]
pub fn derive_canonical_bytes(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    quote! {
        impl #impl_generics ::engine_core::ecs::CanonicalBytes
            for #name #ty_generics #where_clause
        {
            fn canonical_bytes(&self) -> ::core::option::Option<&[u8]> {
                // SAFETY: `Self: Copy + 'static` (enforced by the trait
                // bound) means the value is a self-contained run of bytes
                // we can reinterpret as `&[u8]` for the byte length of the
                // type. The slice borrows from `self`, so its lifetime is
                // bounded by the caller's borrow.
                let bytes: &[u8] = unsafe {
                    ::core::slice::from_raw_parts(
                        (self as *const Self) as *const u8,
                        ::core::mem::size_of::<Self>(),
                    )
                };
                ::core::option::Option::Some(bytes)
            }
        }
    }
    .into()
}
