//! Entity Component System.
//!
//! Entities are IDs ([`Entity`]), components are plain data implementing
//! [`Component`], and systems are functions over a [`World`] scheduled by
//! [`Schedule`]. See spec IV.3.

pub mod entity;
pub mod schedule;
mod storage;
pub mod world;

pub use entity::Entity;
pub use schedule::{Phase, Schedule};
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
/// `#[component(storage = "SparseSet")]`.
pub trait Component: 'static {
    /// The storage backend for this component type.
    const STORAGE: StorageKind = StorageKind::Table;
}
