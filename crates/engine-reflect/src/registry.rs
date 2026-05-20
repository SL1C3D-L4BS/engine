//! [`TypeRegistry`] — a name-keyed table of reflected type metadata.
//!
//! Scene files, the editor, and script bindings refer to types by name. The
//! registry is the lookup that turns a name back into structural metadata.

use crate::reflect::Reflect;
use std::collections::BTreeMap;

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
/// A `BTreeMap` backs it so iteration order is deterministic — a property the
/// Determinism Contract (spec IV.2) wants of every engine-wide table.
#[derive(Debug, Default)]
pub struct TypeRegistry {
    types: BTreeMap<&'static str, TypeInfo>,
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

    /// Looks up a type by name.
    pub fn get(&self, name: &str) -> Option<&TypeInfo> {
        self.types.get(name)
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
