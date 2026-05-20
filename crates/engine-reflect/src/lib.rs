//! `engine-reflect` — runtime reflection and type registration.
//!
//! Level 0 crate (no engine dependencies). See `ENGINE_SPECIFICATION_v2.0.md`
//! Part IV.1.
//!
//! Reflection lets tooling inspect and mutate values whose type is only known
//! at runtime. It backs the editor's reflection-driven INSPECTOR form
//! generator (spec III.2) and the auto-registered ECS script bindings
//! (spec IV.7).
//!
//! - [`Reflect`] — the field-introspection trait, derived by
//!   `#[derive(Reflect)]` from `engine-ecs-macro`.
//! - [`ReflectValue`] — the dynamic value type carried across the boundary.
//! - [`FromReflect`] — fallible conversion back to a concrete type.
//! - [`TypeRegistry`] — a name-keyed table of reflected type metadata.

pub mod reflect;
pub mod registry;
pub mod value;

pub use reflect::Reflect;
pub use registry::{TypeInfo, TypeRegistry, TypeStableId};
pub use value::{FromReflect, ReflectValue};
