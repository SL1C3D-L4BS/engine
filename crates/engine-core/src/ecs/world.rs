//! The [`World`] — the container of all entities, components, and resources.
//!
//! Phase 3 (ADR-031) reshapes the storage layer: Table components live in
//! archetype-grouped columns (see [`super::archetype`]) and SparseSet
//! components live in world-scoped [`SparseColumn`]s keyed by entity index.
//! The user-facing `insert / get / get_mut / remove / for_each / query` API
//! is unchanged from the foundation layer's contract.

use super::Component;
use super::StorageKind;
use super::archetype::{
    AnyVec, AnyVecLayout, ArchetypeId, ArchetypeIndex, ArchetypeSignature, TypeStableId,
};
use super::entity::{Entity, EntityAllocator, EntityLocation};
use super::query::{Mut, Query, WorldQuery};
use super::storage::{AnySparseColumn, SparseColumn};
use crate::collections::{DeterministicHasher, FastHasher, HashMap};
use std::any::{Any, TypeId};

/// The ECS world: entities, their components, and global resources.
///
/// Internally:
///
/// - [`archetypes`](Self::archetype) — the Phase 3 archetype index. Holds
///   every Table component column, grouped by signature.
/// - [`sparse_columns`](Self::sparse_column) — one per SparseSet component
///   type, keyed by entity index.
/// - `resources` — a name-of-`TypeId`-keyed boxed `Any` map. Resources do
///   not participate in archetype iteration; the `TypeId::of::<R>()` calls
///   here are intentional and grandfathered against the ADR-031 CI guard.
pub struct World {
    entities: EntityAllocator,
    archetypes: ArchetypeIndex,
    sparse_columns: HashMap<TypeStableId, Box<dyn AnySparseColumn>, FastHasher>,
    // allow: resources — the resource map is the only TypeId user in this
    // crate; it doesn't participate in the cross-arch frame digest because
    // resources are not iterated by the determinism oracle.
    resources: HashMap<TypeId, Box<dyn Any>, DeterministicHasher>,
}

impl Default for World {
    fn default() -> Self {
        Self {
            entities: EntityAllocator::default(),
            archetypes: ArchetypeIndex::new(),
            sparse_columns: HashMap::with_hasher(FastHasher::new()),
            resources: HashMap::with_hasher(DeterministicHasher::new()),
        }
    }
}

impl World {
    /// Creates an empty world.
    pub fn new() -> Self {
        Self::default()
    }

    /// Spawns a new, component-less entity.
    pub fn spawn(&mut self) -> Entity {
        let entity = self.entities.alloc();
        // Park the new entity in the empty archetype so its location is
        // immediately queryable.
        let empty = self.archetypes.get_mut(ArchetypeId::EMPTY);
        let row = empty.entity_indices.len() as u32;
        empty.entity_indices.push(entity.index());
        self.entities.set_location(
            entity,
            EntityLocation {
                archetype: ArchetypeId::EMPTY,
                row,
            },
        );
        entity
    }

    /// Despawns an entity, removing all of its components.
    ///
    /// Returns `false` if the handle was already stale or dead.
    pub fn despawn(&mut self, entity: Entity) -> bool {
        if !self.entities.is_alive(entity) {
            return false;
        }
        let index = entity.index();
        // 1. Sparse columns (no archetype involvement).
        for column in self.sparse_columns.values_mut() {
            column.erase(index);
        }
        // 2. Archetype: drop the row in place (drop_old = true so the
        // archetype's drop_fn runs).
        let loc = self.entities.location_of(entity).expect("alive entity");
        self.remove_row_dropping(loc);
        self.entities.free(entity)
    }

    /// Returns `true` if `entity` is currently live.
    pub fn is_alive(&self, entity: Entity) -> bool {
        self.entities.is_alive(entity)
    }

    /// The number of live entities.
    pub fn entity_count(&self) -> usize {
        self.entities.live_count()
    }

    /// The number of archetypes currently allocated. Useful for end-of-frame
    /// counters (`ecs_archetype_count` in ADR-033).
    pub fn archetype_count(&self) -> usize {
        self.archetypes.archetype_count()
    }

    /// Attaches (or overwrites) component `T` on `entity`.
    ///
    /// Returns `false` without modifying anything if the handle is stale.
    pub fn insert<T: Component>(&mut self, entity: Entity, value: T) -> bool {
        if !self.entities.is_alive(entity) {
            return false;
        }
        match T::STORAGE {
            StorageKind::Table => self.insert_table(entity, value),
            StorageKind::SparseSet => self.insert_sparse::<T>(entity, value),
        }
    }

    /// Removes component `T` from `entity`, returning it if it was present.
    pub fn remove<T: Component>(&mut self, entity: Entity) -> Option<T> {
        if !self.entities.is_alive(entity) {
            return None;
        }
        match T::STORAGE {
            StorageKind::Table => self.remove_table::<T>(entity),
            StorageKind::SparseSet => self
                .sparse_column_mut_or_insert::<T>()
                .remove(entity.index()),
        }
    }

    /// Borrows component `T` of `entity`.
    pub fn get<T: Component>(&self, entity: Entity) -> Option<&T> {
        if !self.entities.is_alive(entity) {
            return None;
        }
        match T::STORAGE {
            StorageKind::Table => {
                let loc = self.entities.location_of(entity)?;
                let arch = self.archetypes.get(loc.archetype);
                let col = arch.column_index(T::STABLE_ID)?;
                let column = &arch.columns[col];
                // SAFETY: `loc.row` is in-range because the entity is alive
                // and its location was set by the world's own insertion
                // path; `T` matches the column's element layout because the
                // archetype carries `T::STABLE_ID`.
                Some(unsafe { column.get::<T>(loc.row as usize) })
            }
            StorageKind::SparseSet => self
                .sparse_column::<T>()
                .and_then(|c| c.get(entity.index())),
        }
    }

    /// Mutably borrows component `T` of `entity`.
    pub fn get_mut<T: Component>(&mut self, entity: Entity) -> Option<&mut T> {
        if !self.entities.is_alive(entity) {
            return None;
        }
        match T::STORAGE {
            StorageKind::Table => {
                let loc = self.entities.location_of(entity)?;
                let arch = self.archetypes.get_mut(loc.archetype);
                let col = arch.column_index(T::STABLE_ID)?;
                let column = &mut arch.columns[col];
                // SAFETY: same as `get`.
                Some(unsafe { column.get_mut::<T>(loc.row as usize) })
            }
            StorageKind::SparseSet => self
                .sparse_column_mut::<T>()
                .and_then(|c| c.get_mut(entity.index())),
        }
    }

    /// Returns `true` if `entity` has component `T`.
    pub fn contains<T: Component>(&self, entity: Entity) -> bool {
        if !self.entities.is_alive(entity) {
            return false;
        }
        match T::STORAGE {
            StorageKind::Table => {
                let loc = self.entities.location_of(entity);
                loc.is_some_and(|l| {
                    self.archetypes
                        .get(l.archetype)
                        .column_index(T::STABLE_ID)
                        .is_some()
                })
            }
            StorageKind::SparseSet => self
                .sparse_column::<T>()
                .is_some_and(|c| c.contains(entity.index())),
        }
    }

    /// Visits every `(entity, &T)` pair, in ascending entity-index order.
    pub fn for_each<T: Component>(&self, mut f: impl FnMut(Entity, &T)) {
        let mut visits: Vec<(Entity, *const T)> = Vec::new();
        match T::STORAGE {
            StorageKind::Table => {
                for arch in self.archetypes.archetypes.iter() {
                    let Some(col) = arch.column_index(T::STABLE_ID) else {
                        continue;
                    };
                    let column = &arch.columns[col];
                    for row in 0..arch.len() {
                        let idx = arch.entity_indices[row];
                        let Some(entity) = self.entities.entity_at(idx) else {
                            continue;
                        };
                        // SAFETY: layout match by trait bound; `row < len`.
                        let value: &T = unsafe { column.get::<T>(row) };
                        visits.push((entity, value as *const T));
                    }
                }
            }
            StorageKind::SparseSet => {
                let Some(column) = self.sparse_column::<T>() else {
                    return;
                };
                for idx in column.sorted_indices() {
                    let Some(entity) = self.entities.entity_at(idx) else {
                        continue;
                    };
                    let Some(value) = column.get(idx) else {
                        continue;
                    };
                    visits.push((entity, value as *const T));
                }
            }
        }
        // Stable sort by entity index to preserve the documented order
        // (ascending entity index) regardless of archetype layout.
        visits.sort_by_key(|(e, _)| e.index());
        for (entity, ptr) in visits {
            // SAFETY: `ptr` borrows from the world's column storage, which
            // is alive for the duration of `&self`.
            f(entity, unsafe { &*ptr });
        }
    }

    /// Visits every `(entity, &mut T)` pair, in ascending entity-index order.
    pub fn for_each_mut<T: Component>(&mut self, mut f: impl FnMut(Entity, &mut T)) {
        let mut visits: Vec<(Entity, *mut T)> = Vec::new();
        match T::STORAGE {
            StorageKind::Table => {
                for arch in self.archetypes.archetypes.iter_mut() {
                    let Some(col) = arch.column_index(T::STABLE_ID) else {
                        continue;
                    };
                    let column = &mut arch.columns[col];
                    for row in 0..arch.entity_indices.len() {
                        let idx = arch.entity_indices[row];
                        let Some(entity) = self.entities.entity_at(idx) else {
                            continue;
                        };
                        // SAFETY: same as `for_each`. The cast to `*mut T`
                        // is safe because we hold `&mut self`.
                        let value: &mut T = unsafe { column.get_mut::<T>(row) };
                        visits.push((entity, value as *mut T));
                    }
                }
            }
            StorageKind::SparseSet => {
                let entities = &self.entities;
                let Some(boxed) = self.sparse_columns.get_mut(&T::STABLE_ID) else {
                    return;
                };
                let Some(column) = boxed.as_any_mut().downcast_mut::<SparseColumn<T>>() else {
                    return;
                };
                for idx in column.sorted_indices() {
                    let Some(entity) = entities.entity_at(idx) else {
                        continue;
                    };
                    let Some(value) = column.get_mut(idx) else {
                        continue;
                    };
                    visits.push((entity, value as *mut T));
                }
            }
        }
        visits.sort_by_key(|(e, _)| e.index());
        for (entity, ptr) in visits {
            // SAFETY: `ptr` borrows from the world's column storage; we hold
            // `&mut self` so no other reference can alias.
            f(entity, unsafe { &mut *ptr });
        }
    }

    /// Collects every entity that currently has component `T`.
    pub fn entities_with<T: Component>(&self) -> Vec<Entity> {
        let mut out: Vec<Entity> = Vec::new();
        match T::STORAGE {
            StorageKind::Table => {
                for arch in self.archetypes.archetypes.iter() {
                    if arch.column_index(T::STABLE_ID).is_none() {
                        continue;
                    }
                    for &idx in arch.entity_indices.iter() {
                        if let Some(e) = self.entities.entity_at(idx) {
                            out.push(e);
                        }
                    }
                }
            }
            StorageKind::SparseSet => {
                if let Some(column) = self.sparse_column::<T>() {
                    for idx in column.sorted_indices() {
                        if let Some(e) = self.entities.entity_at(idx) {
                            out.push(e);
                        }
                    }
                }
            }
        }
        out.sort_by_key(|e| e.index());
        out
    }

    /// The number of entities holding component `T`.
    pub fn count<T: Component>(&self) -> usize {
        match T::STORAGE {
            StorageKind::Table => self
                .archetypes
                .archetypes
                .iter()
                .filter(|a| a.column_index(T::STABLE_ID).is_some())
                .map(|a| a.len())
                .sum(),
            StorageKind::SparseSet => self.sparse_column::<T>().map(|c| c.len()).unwrap_or(0),
        }
    }

    /// Builds a typed [`Query`] over this world.
    ///
    /// Use `world.query::<&Health>().iter()` to read, or
    /// `world.query::<Mut<Health>>().iter()` to write. Two-component
    /// `(&A, &B)` queries are also supported. See [`query`] for the
    /// `WorldQuery` impls.
    ///
    /// [`query`]: super::query
    pub fn query<'w, Q: WorldQuery<'w>>(&'w self) -> Query<'w, Q> {
        Query::new(self)
    }

    /// Builds a typed mutable query. Convenience wrapper around
    /// [`World::query`] for the `Mut<T>` marker.
    pub fn query_mut<'w, T: Component>(&'w mut self) -> Query<'w, Mut<T>> {
        Query::new(self)
    }

    /// Returns the archetypes whose signature contains every component in
    /// `required`. Caller decides the iteration order.
    pub(crate) fn matching_archetypes(&self, required: &[TypeStableId]) -> Vec<ArchetypeId> {
        let mut out: Vec<ArchetypeId> = Vec::new();
        for arch in self.archetypes.archetypes.iter() {
            if required.iter().all(|id| arch.signature.contains(*id)) {
                out.push(arch.id);
            }
        }
        out
    }

    /// Direct archetype lookup. Used by [`QueryIter`].
    ///
    /// [`QueryIter`]: super::query::QueryIter
    pub(crate) fn archetype(&self, id: ArchetypeId) -> &super::archetype::Archetype {
        self.archetypes.get(id)
    }

    /// Reconstructs the live entity at storage index `idx`. Returns a
    /// fallback dead-handle if the slot is unexpectedly empty — callers
    /// inside the query path will only invoke this for indices materialised
    /// from a live archetype row, so the fallback should never be observed
    /// in practice.
    pub(crate) fn entity_from_index(&self, idx: u32) -> Entity {
        self.entities
            .entity_at(idx)
            .unwrap_or(Entity::from_parts(idx, 0))
    }

    // --- Resources -------------------------------------------------------

    /// Inserts (or replaces) a global resource.
    pub fn insert_resource<R: 'static>(&mut self, resource: R) {
        let id = TypeId::of::<R>(); // allow: resources
        self.resources.insert(id, Box::new(resource));
    }

    /// Borrows a global resource.
    pub fn resource<R: 'static>(&self) -> Option<&R> {
        let id = TypeId::of::<R>(); // allow: resources
        self.resources.get(&id).and_then(|r| r.downcast_ref::<R>())
    }

    /// Mutably borrows a global resource.
    pub fn resource_mut<R: 'static>(&mut self) -> Option<&mut R> {
        let id = TypeId::of::<R>(); // allow: resources
        self.resources
            .get_mut(&id)
            .and_then(|r| r.downcast_mut::<R>())
    }

    /// Removes and returns a global resource.
    pub fn remove_resource<R: 'static>(&mut self) -> Option<R> {
        let id = TypeId::of::<R>(); // allow: resources
        self.resources
            .remove(&id)
            .and_then(|r| r.downcast::<R>().ok())
            .map(|b| *b)
    }

    // --- Table storage helpers -----------------------------------------

    fn insert_table<T: Component>(&mut self, entity: Entity, value: T) -> bool {
        self.archetypes
            .register_layout(T::STABLE_ID, AnyVecLayout::of::<T>());

        let loc = self.entities.location_of(entity).expect("alive entity");
        let from = loc.archetype;
        let from_arch = self.archetypes.get(from);
        // If T is already on the entity, overwrite in place; archetype is
        // unchanged.
        if let Some(col_in_from) = from_arch.column_index(T::STABLE_ID) {
            let column = &mut self.archetypes.get_mut(from).columns[col_in_from];
            // SAFETY: layout match by trait bound; `loc.row` in range.
            let slot: &mut T = unsafe { column.get_mut::<T>(loc.row as usize) };
            *slot = value;
            return true;
        }

        // Otherwise: move row to the destination archetype, then push the
        // new component.
        let to = self.archetypes.dest_with_added(from, T::STABLE_ID);
        let to_row = self.move_row_between_archetypes(from, loc.row as usize, to);

        // Append the new component to its column in `to`.
        let to_arch = self.archetypes.get_mut(to);
        let col_in_to = to_arch
            .column_index(T::STABLE_ID)
            .expect("destination archetype must contain newly-added component");
        // SAFETY: layout was registered above; this column was allocated
        // for `T`.
        let new_row = unsafe { to_arch.columns[col_in_to].push(value) };
        debug_assert_eq!(new_row, to_row);

        // Update the entity's location.
        self.entities.set_location(
            entity,
            EntityLocation {
                archetype: to,
                row: to_row as u32,
            },
        );
        true
    }

    fn remove_table<T: Component>(&mut self, entity: Entity) -> Option<T> {
        let loc = self.entities.location_of(entity)?;
        let from = loc.archetype;
        let from_arch = self.archetypes.get(from);
        // Bail if T was never on the entity.
        from_arch.column_index(T::STABLE_ID)?;

        let to = self.archetypes.dest_with_removed(from, T::STABLE_ID);
        let mut extracted: Vec<u8> = vec![0u8; std::mem::size_of::<T>()];
        let to_row = self.move_row_between_archetypes_inner(
            from,
            loc.row as usize,
            to,
            Some((T::STABLE_ID, &mut extracted)),
        );
        self.entities.set_location(
            entity,
            EntityLocation {
                archetype: to,
                row: to_row as u32,
            },
        );

        // SAFETY: the extract path copied `size_of::<T>()` bytes from a
        // live `T` instance in the column into `extracted` without
        // running drop (the column did `swap_remove_drop` with
        // `drop_old = false`). Re-interpreting those bytes as `T` returns
        // ownership of exactly one `T` to the caller.
        let value: T = unsafe {
            if std::mem::size_of::<T>() != 0 {
                std::ptr::read(extracted.as_ptr() as *const T)
            } else {
                // ZST: zero-sized type has a unique inhabitant; safe to
                // produce one without reading bytes.
                #[allow(clippy::uninit_assumed_init)]
                std::mem::MaybeUninit::<T>::uninit().assume_init()
            }
        };
        Some(value)
    }

    /// Move the row at `from_row` in archetype `from` into a fresh row in
    /// archetype `to`, moving every column that exists in both. Returns
    /// the new row index in `to`.
    fn move_row_between_archetypes(
        &mut self,
        from: ArchetypeId,
        from_row: usize,
        to: ArchetypeId,
    ) -> usize {
        self.move_row_between_archetypes_inner(from, from_row, to, None)
    }

    /// Variant of [`move_row_between_archetypes`] that, when `extract` is
    /// `Some((id, out))`, copies the bytes of the `id` column at `from_row`
    /// into `out` without running drop on the original — used by the
    /// `remove<T>` path to recover ownership of the removed component.
    /// The `id` column must exist in `from` and must *not* exist in `to`
    /// (i.e. the "remove" archetype edge).
    fn move_row_between_archetypes_inner(
        &mut self,
        from: ArchetypeId,
        from_row: usize,
        to: ArchetypeId,
        mut extract: Option<(TypeStableId, &mut [u8])>,
    ) -> usize {
        assert_ne!(from, to);
        let (src, dst) = self.archetypes.pair_mut(from, to);

        // Pre-compute the destination row before we mutate `dst.entity_indices`.
        let new_row = dst.entity_indices.len();
        dst.entity_indices.push(src.entity_indices[from_row]);

        // Move every shared column from src to dst (matched by stable id).
        // Process columns in signature order; src_col_idx must stay aligned
        // with src.signature.
        for src_col_idx in 0..src.signature.len() {
            let src_id = src.signature.as_slice()[src_col_idx];
            if let Some(dst_col_idx) = dst.signature.position(src_id) {
                let src_col = &mut src.columns[src_col_idx];
                let dst_col = &mut dst.columns[dst_col_idx];
                src_col.move_row_into(from_row, dst_col);
            } else if let Some((extract_id, ref mut buf)) = extract
                && extract_id == src_id
            {
                // The "remove" case: this column's row is being lifted out
                // of the world entirely. Copy bytes into the caller's
                // buffer, then swap-remove without running drop.
                // SAFETY: `buf.len()` >= column element size (the caller
                // sized it to `size_of::<T>()`).
                unsafe {
                    src.columns[src_col_idx].take_row_bytes(from_row, buf);
                }
                extract = None;
            } else {
                // Column exists in src but not in dst (and we are not
                // extracting it). Drop the row entirely. This branch is
                // unreachable for the standard add/remove archetype edges
                // because `to.signature` differs from `from.signature` by
                // at most one element.
                src.columns[src_col_idx].swap_remove_drop(from_row, /*drop_old=*/ true);
            }
        }

        // Patch the swap-remove victim in `src`'s entity_indices.
        let last_row = src.entity_indices.len() - 1;
        if from_row != last_row {
            src.entity_indices.swap(from_row, last_row);
        }
        let _ = src.entity_indices.pop().expect("non-empty source");

        if from_row != last_row {
            let moved_idx = src.entity_indices[from_row];
            self.entities.set_location_by_index(
                moved_idx,
                EntityLocation {
                    archetype: from,
                    row: from_row as u32,
                },
            );
        }

        new_row
    }

    /// Drop the row at `loc` entirely, running the per-column drop function.
    fn remove_row_dropping(&mut self, loc: EntityLocation) {
        let arch = self.archetypes.get_mut(loc.archetype);
        let row = loc.row as usize;
        for col in arch.columns.iter_mut() {
            col.swap_remove_drop(row, /*drop_old=*/ true);
        }
        let last_row = arch.entity_indices.len() - 1;
        if row != last_row {
            arch.entity_indices.swap(row, last_row);
        }
        arch.entity_indices.pop();
        if row != last_row {
            let moved_idx = arch.entity_indices[row];
            self.entities.set_location_by_index(
                moved_idx,
                EntityLocation {
                    archetype: loc.archetype,
                    row: row as u32,
                },
            );
        }
    }

    // --- Sparse storage helpers ----------------------------------------

    fn insert_sparse<T: Component>(&mut self, entity: Entity, value: T) -> bool {
        let column = self.sparse_column_mut_or_insert::<T>();
        column.insert(entity.index(), value);
        true
    }

    fn sparse_column<T: Component>(&self) -> Option<&SparseColumn<T>> {
        self.sparse_columns
            .get(&T::STABLE_ID)
            .and_then(|c| c.as_any().downcast_ref::<SparseColumn<T>>())
    }

    fn sparse_column_mut<T: Component>(&mut self) -> Option<&mut SparseColumn<T>> {
        self.sparse_columns
            .get_mut(&T::STABLE_ID)
            .and_then(|c| c.as_any_mut().downcast_mut::<SparseColumn<T>>())
    }

    fn sparse_column_mut_or_insert<T: Component>(&mut self) -> &mut SparseColumn<T> {
        let id = T::STABLE_ID;
        if !self.sparse_columns.contains_key(&id) {
            self.sparse_columns
                .insert(id, Box::new(SparseColumn::<T>::new()));
        }
        self.sparse_columns
            .get_mut(&id)
            .expect("just inserted")
            .as_any_mut()
            .downcast_mut::<SparseColumn<T>>()
            .expect("concrete type matches stable id")
    }
}

// SparseColumn::len helper used by World::count.
impl<T> SparseColumn<T> {
    pub(crate) fn len(&self) -> usize {
        self.sorted_indices().len()
    }
}

// AnyVec needs to be Sized; it is. Just bring it into scope here so the
// trait paths above resolve.
use std::marker::PhantomData;
#[allow(dead_code)]
type _BringAnyVecIntoScope = PhantomData<AnyVec>;
#[allow(dead_code)]
type _BringSignatureIntoScope = PhantomData<ArchetypeSignature>;
