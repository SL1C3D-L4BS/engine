//! Entity Component System.
//!
//! Entities are IDs ([`Entity`]), components are plain data implementing
//! [`Component`], and systems are functions over a [`World`] scheduled by
//! [`Schedule`]. See spec IV.3.
//!
//! Phase 3 (ADR-031) replaces the flat per-component column layout with an
//! archetype index — entities are physically grouped by their Table-component
//! signature, SparseSet components remain world-scoped sidecars per ADR-002.
//! The user-facing API ([`World::insert`], [`World::get`], [`World::for_each`])
//! is unchanged; what changed is the storage layout underneath. The new
//! [`Query`] API is the archetype-aware iteration primitive for the
//! cross-component hot path.

pub mod archetype;
pub mod entity;
pub mod query;
pub mod schedule;
mod storage;
pub mod type_id;
pub mod world;

pub use archetype::{ArchetypeId, ArchetypeSignature};
pub use entity::Entity;
pub use query::{Mut, Query};
pub use schedule::{Phase, Schedule};
pub use type_id::{TypeStableId, stable_id_of};
pub use world::World;

/// The storage backend a component type uses (spec IV.3, ADR-002).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorageKind {
    /// Archetype-grouped, contiguous — the cache-friendly hot path. Default.
    Table,
    /// Sparse/dense pair — O(1) insert and remove, best for tag-like and
    /// churn-heavy components.
    SparseSet,
}

/// A type that can be attached to an entity as a component.
///
/// Derive it with `#[derive(Component)]`; opt into sparse storage with
/// `#[component(storage = "SparseSet")]`. The derive emits both [`STORAGE`]
/// and [`STABLE_ID`] — the latter is computed at macro-expansion time from
/// `BLAKE3(crate_name || "::" || ident)` so the archetype signature is
/// bit-identical across architectures (ADR-031).
///
/// [`STORAGE`]: Component::STORAGE
/// [`STABLE_ID`]: Component::STABLE_ID
pub trait Component: 'static {
    /// The storage backend for this component type.
    const STORAGE: StorageKind = StorageKind::Table;
    /// Cross-architecture-stable type identifier (ADR-031). The archetype
    /// index keys signatures by this; never by `std::any::TypeId`.
    const STABLE_ID: TypeStableId;
}
