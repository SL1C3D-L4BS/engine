//! [`TypeRegistry`] — a name-keyed table of reflected type metadata, plus the
//! cross-architecture-stable [`TypeStableId`] used by the ECS archetype index
//! (ADR-031).

use crate::reflect::Reflect;
use std::collections::BTreeMap;

/// A 64-bit stable identifier for a Rust type.
///
/// Computed by `#[derive(Component)]` (ADR-031) as the first eight bytes of
/// `BLAKE3(crate_name || "::" || type_ident)` — bit-identical across runs,
/// builds, and architectures. The ECS archetype index keys signatures by
/// `TypeStableId`, never by `std::any::TypeId` (which is process-private and
/// would break cross-arch determinism, ADR-013).
///
/// The `u64` is public on purpose: serialised scene files name component
/// types by stable id, and exposing the integer makes the wire format
/// inspectable without going through the runtime registry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TypeStableId(pub u64);

impl TypeStableId {
    /// The 64-bit value.
    #[inline]
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for TypeStableId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TypeStableId(0x{:016x})", self.0)
    }
}

/// Structural metadata for one reflected type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeInfo {
    /// The type's name.
    pub name: &'static str,
    /// Field names in declaration order.
    pub field_names: Vec<&'static str>,
}

/// A registry of reflected types, keyed by type name.
///
/// A [`BTreeMap`] backs the primary table so iteration order is deterministic
/// — a property the Determinism Contract (spec IV.2) wants of every
/// engine-wide table. A parallel `BTreeMap<TypeStableId, &'static str>`
/// supports back-lookup by id without changing the existing name-keyed API
/// (ADR-031).
#[derive(Debug, Default)]
pub struct TypeRegistry {
    types: BTreeMap<&'static str, TypeInfo>,
    by_stable_id: BTreeMap<TypeStableId, &'static str>,
}

impl TypeRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers the type of `sample`, reading its metadata via reflection.
    ///
    /// Registering the same type again overwrites the previous entry.
    pub fn register(&mut self, sample: &dyn Reflect) {
        let info = TypeInfo {
            name: sample.type_name(),
            field_names: sample.field_names(),
        };
        self.types.insert(info.name, info);
    }

    /// Associates `name` with `stable_id` in the back-lookup table.
    ///
    /// Phase 3 (ADR-031): components register their `TypeStableId` here so
    /// scene loaders can resolve `0x…` back to a name string. The forward
    /// (name-keyed) registry is untouched.
    pub fn register_stable_id(&mut self, name: &'static str, stable_id: TypeStableId) {
        self.by_stable_id.insert(stable_id, name);
    }

    /// Looks up a type by name.
    pub fn get(&self, name: &str) -> Option<&TypeInfo> {
        self.types.get(name)
    }

    /// Looks up a type name by its [`TypeStableId`].
    pub fn name_of(&self, stable_id: TypeStableId) -> Option<&'static str> {
        self.by_stable_id.get(&stable_id).copied()
    }

    /// Returns `true` if a type with the given name is registered.
    pub fn contains(&self, name: &str) -> bool {
        self.types.contains_key(name)
    }

    /// The number of registered types.
    pub fn len(&self) -> usize {
        self.types.len()
    }

    /// Returns `true` if no types are registered.
    pub fn is_empty(&self) -> bool {
        self.types.is_empty()
    }

    /// Iterates registered types in deterministic (name-sorted) order.
    pub fn iter(&self) -> impl Iterator<Item = &TypeInfo> {
        self.types.values()
    }
}
