//! Archetype-aware query iteration (ADR-031).
//!
//! The Phase 3 typed query surface is intentionally minimal: a small
//! [`WorldQuery`] trait, plus implementations for `&T`, `&mut T`, and
//! pairwise tuples thereof. A query walks matching archetypes in ascending
//! [`ArchetypeId`] order, rows in ascending order; that fixes the iteration
//! sequence the determinism contract requires.
//!
//! The richer DSL (`With<T>`, `Without<T>`, multi-component tuples beyond
//! pairs) is deferred to Phase 4+ — the plan calls out a
//! `SystemParam`-driven query DSL as out of scope here. What ships is
//! enough to thread the cross-cutting hot loops of Phase 3 (the
//! 1M-entity benchmark, the replay-parity oracle) through a real
//! archetype-aware iterator.
//!
//! The iterators here borrow from the [`World`]; the long-lived archetype
//! arrays do not move while a query iterator is alive.

use super::Component;
use super::archetype::{Archetype, ArchetypeId, TypeStableId};
use super::entity::Entity;
use super::world::World;

/// Joint trait for typed archetype queries. Implementors fill in the small
/// per-archetype hooks; [`QueryIter`] does the archetype walk.
pub trait WorldQuery<'w>: Sized {
    /// The item the iterator yields.
    type Item;
    /// Per-archetype state cached after the column lookups succeed (e.g.
    /// the raw pointer + stride for each column).
    type ArchState;

    /// Component ids this query reads or writes. Used by the archetype
    /// filter to skip archetypes that don't carry every requested
    /// component.
    fn required_components() -> Vec<TypeStableId>;

    /// Builds the per-archetype fetch state, or `None` to skip the
    /// archetype. `archetype` is borrowed from `world` for the duration of
    /// the iterator.
    ///
    /// # Safety
    ///
    /// The returned state borrows from `world`; the caller (the
    /// [`QueryIter`]) is responsible for ensuring the state is only used
    /// while the borrow is live.
    unsafe fn build_arch_state(world: &'w World, archetype: &Archetype) -> Option<Self::ArchState>;

    /// Fetches row `row` from the cached archetype state.
    ///
    /// # Safety
    ///
    /// `state` must come from this archetype's [`build_arch_state`]; `row`
    /// must be a valid row index in the archetype's columns; and no other
    /// aliasing reference to the same row must be live for queries that
    /// produce mutable references.
    unsafe fn fetch(state: &mut Self::ArchState, row: usize) -> Self::Item;
}

/// Iterator over `Q::Item` for every row in every matching archetype.
pub struct QueryIter<'w, Q: WorldQuery<'w>> {
    world: &'w World,
    matching_archetypes: Vec<ArchetypeId>,
    next_arch: usize,
    current: Option<(ArchetypeId, Q::ArchState, usize, usize)>,
}

impl<'w, Q: WorldQuery<'w>> QueryIter<'w, Q> {
    /// Builds a fresh iterator. The set of matching archetypes is
    /// materialised eagerly so the iterator is `Send`-compatible regardless
    /// of the archetype index's hashmap iteration order.
    pub fn new(world: &'w World) -> Self {
        let required = Q::required_components();
        let mut matching = world.matching_archetypes(&required);
        matching.sort_unstable();
        Self {
            world,
            matching_archetypes: matching,
            next_arch: 0,
            current: None,
        }
    }
}

impl<'w, Q: WorldQuery<'w>> Iterator for QueryIter<'w, Q> {
    type Item = Q::Item;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some((_, state, row, len)) = self.current.as_mut() {
                if *row < *len {
                    let r = *row;
                    *row += 1;
                    // SAFETY: `state` is the active archetype's fetch state
                    // and `r` is in `0..len` for that archetype.
                    return Some(unsafe { Q::fetch(state, r) });
                }
                self.current = None;
            }
            if self.next_arch >= self.matching_archetypes.len() {
                return None;
            }
            let aid = self.matching_archetypes[self.next_arch];
            self.next_arch += 1;
            let arch = self.world.archetype(aid);
            // SAFETY: the world reference outlives the iterator; the
            // archetype reference also outlives the per-archetype state
            // because the world's archetype vector is not reallocated while
            // an immutable borrow is held.
            let state = unsafe { Q::build_arch_state(self.world, arch) };
            if let Some(state) = state {
                let len = arch.len();
                self.current = Some((aid, state, 0, len));
            }
        }
    }
}

/// A typed archetype query. Built via [`World::query`] / [`World::query_mut`].
pub struct Query<'w, Q: WorldQuery<'w>> {
    world: &'w World,
    _marker: std::marker::PhantomData<fn() -> Q>,
}

impl<'w, Q: WorldQuery<'w>> Query<'w, Q> {
    /// Builds a query over `world`.
    pub fn new(world: &'w World) -> Self {
        Self {
            world,
            _marker: std::marker::PhantomData,
        }
    }

    /// Iterates `Q::Item` over every row in every matching archetype.
    pub fn iter(self) -> QueryIter<'w, Q> {
        QueryIter::new(self.world)
    }
}

// --- Single-component queries ----------------------------------------------

/// Per-archetype state for a single-component read.
#[doc(hidden)]
pub struct ReadState<T: Component> {
    base: *const T,
    /// `row → entity index` for the archetype, so iteration can yield
    /// `(Entity, &T)`.
    entity_indices: *const u32,
    /// Borrowed reference to the world's entity allocator — used to
    /// resolve the live generation of a row's entity.
    world: *const World,
}

impl<'w, T: Component> WorldQuery<'w> for &'w T {
    type Item = (Entity, &'w T);
    type ArchState = ReadState<T>;

    fn required_components() -> Vec<TypeStableId> {
        vec![T::STABLE_ID]
    }

    unsafe fn build_arch_state(world: &'w World, archetype: &Archetype) -> Option<Self::ArchState> {
        let col = archetype.column_index(T::STABLE_ID)?;
        let column = &archetype.columns[col];
        let base = column_data_ptr::<T>(column);
        Some(ReadState {
            base,
            entity_indices: archetype.entity_indices.as_ptr(),
            world: world as *const World,
        })
    }

    unsafe fn fetch(state: &mut Self::ArchState, row: usize) -> Self::Item {
        // SAFETY: `row` is in range for the archetype's column (caller
        // upholds this). `base` was derived from the column's allocation,
        // which is alive for the lifetime of the world.
        let value: &T = unsafe { &*state.base.add(row) };
        let idx = unsafe { *state.entity_indices.add(row) };
        let entity = unsafe { (*state.world).entity_from_index(idx) };
        (entity, value)
    }
}

/// Per-archetype state for a single-component write.
#[doc(hidden)]
pub struct WriteState<T: Component> {
    base: *mut T,
    entity_indices: *const u32,
    world: *const World,
}

/// Marker that yields `&mut T` for component `T`.
pub struct Mut<T>(std::marker::PhantomData<fn() -> T>);

impl<'w, T: Component> WorldQuery<'w> for Mut<T> {
    type Item = (Entity, &'w mut T);
    type ArchState = WriteState<T>;

    fn required_components() -> Vec<TypeStableId> {
        vec![T::STABLE_ID]
    }

    unsafe fn build_arch_state(world: &'w World, archetype: &Archetype) -> Option<Self::ArchState> {
        let col = archetype.column_index(T::STABLE_ID)?;
        let column = &archetype.columns[col];
        let base = column_data_ptr_mut::<T>(column);
        Some(WriteState {
            base,
            entity_indices: archetype.entity_indices.as_ptr(),
            world: world as *const World,
        })
    }

    unsafe fn fetch(state: &mut Self::ArchState, row: usize) -> Self::Item {
        // SAFETY: see `&T` impl. The mutable variant relies on the world
        // being borrowed mutably for the lifetime of the query iterator.
        let value: &mut T = unsafe { &mut *state.base.add(row) };
        let idx = unsafe { *state.entity_indices.add(row) };
        let entity = unsafe { (*state.world).entity_from_index(idx) };
        (entity, value)
    }
}

// --- Two-component queries -------------------------------------------------

/// Per-archetype state for a `(&A, &B)` query.
#[doc(hidden)]
pub struct ReadRead<A: Component, B: Component> {
    a: *const A,
    b: *const B,
    entity_indices: *const u32,
    world: *const World,
}

impl<'w, A: Component, B: Component> WorldQuery<'w> for (&'w A, &'w B) {
    type Item = (Entity, &'w A, &'w B);
    type ArchState = ReadRead<A, B>;

    fn required_components() -> Vec<TypeStableId> {
        vec![A::STABLE_ID, B::STABLE_ID]
    }

    unsafe fn build_arch_state(world: &'w World, archetype: &Archetype) -> Option<Self::ArchState> {
        let ca = archetype.column_index(A::STABLE_ID)?;
        let cb = archetype.column_index(B::STABLE_ID)?;
        let a = column_data_ptr::<A>(&archetype.columns[ca]);
        let b = column_data_ptr::<B>(&archetype.columns[cb]);
        Some(ReadRead {
            a,
            b,
            entity_indices: archetype.entity_indices.as_ptr(),
            world: world as *const World,
        })
    }

    unsafe fn fetch(state: &mut Self::ArchState, row: usize) -> Self::Item {
        let a: &A = unsafe { &*state.a.add(row) };
        let b: &B = unsafe { &*state.b.add(row) };
        let idx = unsafe { *state.entity_indices.add(row) };
        let entity = unsafe { (*state.world).entity_from_index(idx) };
        (entity, a, b)
    }
}

// --- raw pointer helpers ---------------------------------------------------

fn column_data_ptr<T: Component>(column: &super::archetype::AnyVec) -> *const T {
    if column.is_empty() {
        // Empty column: the data pointer can be null when no rows exist
        // yet; the `fetch` path never reaches it because the iterator
        // short-circuits on row count.
        return std::ptr::null();
    }
    // SAFETY: column was allocated for `T` (the layout matches by trait
    // bound). Calling `get` on row 0 gives a `&T` we can immediately
    // convert to a raw pointer.
    let r: &T = unsafe { column.get::<T>(0) };
    r as *const T
}

fn column_data_ptr_mut<T: Component>(column: &super::archetype::AnyVec) -> *mut T {
    column_data_ptr::<T>(column) as *mut T
}
