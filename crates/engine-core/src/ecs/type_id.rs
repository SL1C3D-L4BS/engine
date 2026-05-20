//! Stable type identity for ECS components (ADR-031).
//!
//! The archetype index keys signatures by [`TypeStableId`], never by
//! `std::any::TypeId` — `TypeId` is process-private (its bit pattern can
//! change between builds and is not guaranteed to match across the two
//! architectures the determinism oracle runs on), and that's exactly the
//! cross-arch invariant the contract names (spec IV.2 / ADR-013).
//!
//! `TypeStableId` is computed at `#[derive(Component)]` expansion time as
//! `BLAKE3(crate_name || "::" || ident)[..8]`, emitted as a literal `u64`,
//! and stored on the [`Component`] trait as `const STABLE_ID`. That makes
//! the id available in `const` contexts — archetype signatures intern at
//! constant cost, and the proc-macro is the single source of truth for the
//! hashed string.
//!
//! [`Component`]: super::Component

pub use engine_reflect::TypeStableId;

use super::Component;

/// The [`TypeStableId`] of component type `T`.
///
/// Thin generic accessor over `T::STABLE_ID` — useful where a generic
/// function needs the id at runtime and the caller is already bounded by
/// [`Component`].
#[inline]
pub const fn stable_id_of<T: Component>() -> TypeStableId {
    T::STABLE_ID
}
