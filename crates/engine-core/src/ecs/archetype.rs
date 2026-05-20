//! Archetype storage — entities physically grouped by their Table-component
//! signature (ADR-031).
//!
//! An [`Archetype`] owns one row per entity that shares its signature: a flat
//! `Vec<u32>` of entity indices plus one [`AnyVec`] per component column. A
//! query that asks for `(&A, &B)` walks every archetype whose signature
//! contains both `A::STABLE_ID` and `B::STABLE_ID`, in ascending
//! [`ArchetypeId`] order, and streams the matching column slices — that's the
//! cache-friendly loop the 1M-entity milestone depends on.
//!
//! SparseSet components are *not* part of the signature; they live in
//! world-scoped sparse columns (see [`super::storage::SparseColumn`]) and are
//! joined to archetype iteration by sparse lookup. ADR-002's hybrid model is
//! unchanged.

use crate::collections::{DeterministicHasher, FastHasher, HashMap};
use std::alloc::{Layout, alloc, dealloc};
use std::ptr::NonNull;

pub use super::type_id::TypeStableId;

/// Identifier of an archetype within a world.
///
/// Allocated densely from `0` as new signatures are interned. Bit-stable
/// across runs because the signature → id interning table uses
/// [`DeterministicHasher`] (cross-arch reproducible probe order, ADR-028).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ArchetypeId(pub u32);

impl ArchetypeId {
    /// The empty archetype — every world allocates this one on construction
    /// so a freshly-spawned, component-less entity has a valid location to
    /// point at without an extra interning step.
    pub const EMPTY: ArchetypeId = ArchetypeId(0);

    /// The raw id.
    #[inline]
    pub const fn index(self) -> u32 {
        self.0
    }
}

/// The Table-component signature of an archetype: a sorted, deduplicated
/// `Vec<TypeStableId>`. Sorting is essential — two equivalent component sets
/// must intern to the same archetype regardless of insert order, and the
/// determinism contract requires the hash key to be input-order-independent
/// (ADR-031).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ArchetypeSignature {
    /// Component ids in ascending order.
    ids: Vec<TypeStableId>,
}

impl ArchetypeSignature {
    /// The empty signature.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Builds a signature from an unsorted list, sorting and deduplicating.
    pub fn from_unsorted(mut ids: Vec<TypeStableId>) -> Self {
        ids.sort_unstable();
        ids.dedup();
        Self { ids }
    }

    /// The ids in ascending order.
    pub fn as_slice(&self) -> &[TypeStableId] {
        &self.ids
    }

    /// The number of distinct Table components in the signature.
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    /// Returns `true` if the signature is empty.
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// The position of `id` in [`as_slice`](Self::as_slice), if present.
    pub fn position(&self, id: TypeStableId) -> Option<usize> {
        self.ids.binary_search(&id).ok()
    }

    /// `true` if the signature contains `id`.
    pub fn contains(&self, id: TypeStableId) -> bool {
        self.position(id).is_some()
    }

    /// Returns a new signature that is `self ∪ {id}`. If `id` is already in
    /// the signature, returns a clone.
    pub fn with_added(&self, id: TypeStableId) -> Self {
        let mut ids = self.ids.clone();
        if let Err(pos) = ids.binary_search(&id) {
            ids.insert(pos, id);
        }
        Self { ids }
    }

    /// Returns a new signature that is `self \ {id}`. If `id` is not in the
    /// signature, returns a clone.
    pub fn with_removed(&self, id: TypeStableId) -> Self {
        let mut ids = self.ids.clone();
        if let Ok(pos) = ids.binary_search(&id) {
            ids.remove(pos);
        }
        Self { ids }
    }
}

/// A type-erased contiguous column: `len` values of a single component type
/// packed back-to-back, with the per-element drop function carried alongside
/// the layout.
///
/// This is the storage primitive each archetype uses for its Table columns.
/// It owns the buffer; on drop, every live element is dropped in place and
/// the backing allocation is returned to the system allocator.
pub(crate) struct AnyVec {
    /// Per-element layout (size, align, drop_fn). Captured at construction
    /// so the column can drop and reallocate without seeing the concrete `T`
    /// again.
    layout: AnyVecLayout,
    /// Backing allocation. Null when `cap == 0`.
    data: *mut u8,
    /// Number of live elements.
    len: usize,
    /// Allocated element count (`data` is `cap * layout.size` bytes).
    cap: usize,
}

/// Per-element layout shared across every operation on an [`AnyVec`].
#[derive(Clone, Copy)]
pub(crate) struct AnyVecLayout {
    /// `size_of::<T>()`.
    pub(crate) size: usize,
    /// `align_of::<T>()`.
    pub(crate) align: usize,
    /// In-place drop function: `drop_in_place::<T>(ptr as *mut T)`.
    pub(crate) drop_fn: unsafe fn(*mut u8),
}

impl AnyVecLayout {
    /// Builds a layout descriptor for component type `T`.
    pub fn of<T: 'static>() -> Self {
        Self {
            size: std::mem::size_of::<T>(),
            align: std::mem::align_of::<T>(),
            drop_fn: drop_in_place_typed::<T>,
        }
    }
}

unsafe fn drop_in_place_typed<T>(ptr: *mut u8) {
    // SAFETY: caller guarantees `ptr` points to a fully-initialised `T`
    // owned by this column (the column tracks `len` and only invokes this
    // function on indices < `len`).
    unsafe {
        std::ptr::drop_in_place(ptr as *mut T);
    }
}

impl AnyVec {
    /// Empty column for the given layout. Allocates lazily.
    pub fn new(layout: AnyVecLayout) -> Self {
        Self {
            layout,
            data: std::ptr::null_mut(),
            len: 0,
            cap: 0,
        }
    }

    /// The layout descriptor of this column's elements.
    #[allow(dead_code)]
    pub(crate) fn element_layout(&self) -> AnyVecLayout {
        self.layout
    }

    /// The number of live elements.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns `true` if the column has no live elements.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Reserves room for at least `additional` more elements.
    fn reserve(&mut self, additional: usize) {
        let need = self
            .len
            .checked_add(additional)
            .expect("AnyVec length overflowed usize");
        if need <= self.cap {
            return;
        }
        let mut new_cap = if self.cap == 0 { 4 } else { self.cap * 2 };
        while new_cap < need {
            new_cap = new_cap
                .checked_mul(2)
                .expect("AnyVec capacity overflowed usize");
        }
        self.grow_to(new_cap);
    }

    fn grow_to(&mut self, new_cap: usize) {
        debug_assert!(new_cap >= self.cap);
        // ZST short-circuit: zero-sized columns never touch the allocator;
        // `data` stays `dangling()`-equivalent (we use null, the read paths
        // never dereference for ZSTs).
        if self.layout.size == 0 {
            self.cap = new_cap;
            return;
        }
        let new_bytes = new_cap
            .checked_mul(self.layout.size)
            .expect("AnyVec byte capacity overflowed usize");
        let new_layout =
            Layout::from_size_align(new_bytes, self.layout.align).expect("AnyVec layout overflow");
        let new_ptr = if self.cap == 0 {
            // SAFETY: layout has size > 0 by the branch above.
            unsafe { alloc(new_layout) }
        } else {
            let old_bytes = self.cap * self.layout.size;
            let old_layout = Layout::from_size_align(old_bytes, self.layout.align)
                .expect("AnyVec old layout overflow");
            // SAFETY: `data` was allocated with `old_layout`. We grow into a
            // fresh allocation and memcpy the live prefix; the old block is
            // freed below.
            let np = unsafe { alloc(new_layout) };
            if !np.is_null() {
                unsafe {
                    std::ptr::copy_nonoverlapping(self.data, np, self.len * self.layout.size);
                    dealloc(self.data, old_layout);
                }
            } else {
                // Allocation failure → leave the existing buffer in place
                // and fall through to the null check below, which aborts.
                unsafe {
                    dealloc(self.data, old_layout);
                }
            }
            np
        };
        if new_ptr.is_null() {
            std::alloc::handle_alloc_error(new_layout);
        }
        self.data = new_ptr;
        self.cap = new_cap;
    }

    /// Appends a typed value. Returns the row index of the new element.
    ///
    /// # Safety
    ///
    /// `T` must match the column's element layout (same size, same align,
    /// same drop semantics).
    pub unsafe fn push<T>(&mut self, value: T) -> usize {
        debug_assert_eq!(std::mem::size_of::<T>(), self.layout.size);
        debug_assert_eq!(std::mem::align_of::<T>(), self.layout.align);
        self.reserve(1);
        let row = self.len;
        if self.layout.size != 0 {
            // SAFETY: capacity was just reserved; `data + row * size` is
            // within the allocation and the slot is uninitialised.
            unsafe {
                let dst = self.data.add(row * self.layout.size) as *mut T;
                dst.write(value);
            }
        } else {
            // ZST: no bytes to write; the count alone tracks logical
            // existence. The value is moved here and dropped — that's how
            // `Vec<()>` handles ZSTs too.
            drop(value);
        }
        self.len += 1;
        row
    }

    /// Moves the bytes of element `row` into `dst` (which gets a fresh row),
    /// then swap-removes `row` from `self`.
    ///
    /// `dst` and `self` must share the same element layout.
    pub(crate) fn move_row_into(&mut self, row: usize, dst: &mut AnyVec) {
        debug_assert_eq!(self.layout.size, dst.layout.size);
        debug_assert_eq!(self.layout.align, dst.layout.align);
        debug_assert!(row < self.len);
        dst.reserve(1);
        if self.layout.size != 0 {
            // SAFETY: `row` is in-range; `dst.len` is the next free row in
            // `dst`. Both pointers point into disjoint allocations (`self`
            // and `dst` are distinct columns); we copy `size` bytes from
            // one to the other.
            unsafe {
                let src = self.data.add(row * self.layout.size);
                let dest = dst.data.add(dst.len * self.layout.size);
                std::ptr::copy_nonoverlapping(src, dest, self.layout.size);
            }
        }
        dst.len += 1;
        self.swap_remove_drop(row, /*drop_old=*/ false);
    }

    /// Swap-removes element `row`. If `drop_old` is true, the element being
    /// removed is dropped first; otherwise the bytes are abandoned (used
    /// when the bytes were just *moved out* via [`move_row_into`]).
    pub(crate) fn swap_remove_drop(&mut self, row: usize, drop_old: bool) {
        debug_assert!(row < self.len);
        let size = self.layout.size;
        if drop_old && size != 0 {
            // SAFETY: `row` is a live element; the layout's drop_fn is the
            // typed `drop_in_place` for the original element type.
            unsafe {
                (self.layout.drop_fn)(self.data.add(row * size));
            }
        }
        let last = self.len - 1;
        if last != row && size != 0 {
            // SAFETY: both `row` and `last` are in-range slots in the same
            // allocation; copy_nonoverlapping is safe even if they overlap
            // (since `row != last`).
            unsafe {
                let src = self.data.add(last * size);
                let dst = self.data.add(row * size);
                std::ptr::copy_nonoverlapping(src, dst, size);
            }
        }
        self.len -= 1;
    }

    /// Borrows element `row` as `&T`.
    ///
    /// # Safety
    ///
    /// `T` must match the column's element layout and `row` must be in
    /// `0..len`.
    pub unsafe fn get<T>(&self, row: usize) -> &T {
        debug_assert!(row < self.len);
        debug_assert_eq!(std::mem::size_of::<T>(), self.layout.size);
        // SAFETY: caller upholds layout match; `row < len` upholds bounds.
        unsafe { &*(self.data.add(row * self.layout.size) as *const T) }
    }

    /// Mutable counterpart to [`get`](Self::get).
    ///
    /// # Safety
    ///
    /// Same as [`get`](Self::get).
    pub unsafe fn get_mut<T>(&mut self, row: usize) -> &mut T {
        debug_assert!(row < self.len);
        debug_assert_eq!(std::mem::size_of::<T>(), self.layout.size);
        // SAFETY: caller upholds layout match; `row < len` upholds bounds.
        unsafe { &mut *(self.data.add(row * self.layout.size) as *mut T) }
    }

    /// Read the full bytes of element `row` into `out` and swap-remove the
    /// slot without running drop (the bytes are now logically owned by
    /// `out`).
    ///
    /// # Safety
    ///
    /// `out` must have `len >= self.layout.size`. The caller is responsible
    /// for treating the bytes as the original `T` afterwards.
    pub(crate) unsafe fn take_row_bytes(&mut self, row: usize, out: &mut [u8]) {
        debug_assert!(row < self.len);
        let size = self.layout.size;
        debug_assert!(out.len() >= size);
        if size != 0 {
            unsafe {
                let src = self.data.add(row * size);
                std::ptr::copy_nonoverlapping(src, out.as_mut_ptr(), size);
            }
        }
        self.swap_remove_drop(row, /*drop_old=*/ false);
    }
}

impl Drop for AnyVec {
    fn drop(&mut self) {
        let size = self.layout.size;
        if size != 0 {
            for i in 0..self.len {
                // SAFETY: every `i < len` slot holds a live element by
                // construction; the layout's drop_fn drops in place.
                unsafe {
                    (self.layout.drop_fn)(self.data.add(i * size));
                }
            }
            if self.cap != 0 && !self.data.is_null() {
                let bytes = self.cap * size;
                let layout =
                    Layout::from_size_align(bytes, self.layout.align).expect("AnyVec drop layout");
                // SAFETY: `data` was allocated with this exact layout in
                // `grow_to`; we are the sole owner.
                unsafe {
                    dealloc(self.data, layout);
                }
            }
        }
        self.len = 0;
        self.cap = 0;
        self.data = std::ptr::null_mut();
    }
}

// SAFETY: `AnyVec` owns its allocation. `Send`/`Sync` are safe iff the
// elements it stores are themselves `Send`/`Sync`; the trait bounds the
// caller must satisfy when constructing a column for type `T` cover that.
unsafe impl Send for AnyVec {}
unsafe impl Sync for AnyVec {}

/// A row in an [`Archetype`].
///
/// `Default` is provided for use as a placeholder in the entity-location
/// table before an entity is moved into a real archetype.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ArchetypeRow(pub u32);

/// One archetype: a homogenously-typed slice of the world's entities, plus
/// the Table-component columns they share.
///
/// Public so [`super::query`] can name it in `WorldQuery` impls; the field
/// access is `pub(crate)` so external code can't reach into the columns.
pub struct Archetype {
    pub(crate) id: ArchetypeId,
    pub(crate) signature: ArchetypeSignature,
    /// Columns parallel to `signature.as_slice()`. Position `i` holds the
    /// data for component `signature.as_slice()[i]`.
    pub(crate) columns: Vec<AnyVec>,
    /// `row → entity index` so a query can recover the entity for a given
    /// row.
    pub(crate) entity_indices: Vec<u32>,
}

impl Archetype {
    /// Builds an empty archetype with the given signature and (per-component)
    /// layouts. `layouts.len() == signature.len()` must hold.
    pub(crate) fn new(
        id: ArchetypeId,
        signature: ArchetypeSignature,
        layouts: Vec<AnyVecLayout>,
    ) -> Self {
        assert_eq!(layouts.len(), signature.len());
        let columns = layouts.into_iter().map(AnyVec::new).collect();
        Self {
            id,
            signature,
            columns,
            entity_indices: Vec::new(),
        }
    }

    /// This archetype's id.
    pub fn id(&self) -> ArchetypeId {
        self.id
    }

    /// This archetype's signature.
    pub fn signature(&self) -> &ArchetypeSignature {
        &self.signature
    }

    /// Number of rows currently in the archetype.
    pub fn len(&self) -> usize {
        self.entity_indices.len()
    }

    /// `true` if no entities currently live in this archetype.
    pub fn is_empty(&self) -> bool {
        self.entity_indices.is_empty()
    }

    /// Returns the column index for `id` in this archetype, if present.
    pub fn column_index(&self, id: TypeStableId) -> Option<usize> {
        self.signature.position(id)
    }
}

/// The world's archetype index: every archetype plus the lookup tables that
/// turn `signature` into [`ArchetypeId`] and `(from, type_added)` into
/// `to` (the adjacency cache).
pub(crate) struct ArchetypeIndex {
    /// All archetypes, indexed by `ArchetypeId.0 as usize`.
    pub(crate) archetypes: Vec<Archetype>,
    /// Signature interning. [`DeterministicHasher`] so the order in which
    /// archetypes are minted is cross-arch reproducible (ADR-028, ADR-031).
    by_signature: HashMap<ArchetypeSignature, ArchetypeId, DeterministicHasher>,
    /// Per-component layout cache. The first time a column for `T` is
    /// allocated we record the layout here; subsequent archetypes that hold
    /// `T` reuse the same descriptor.
    layouts: HashMap<TypeStableId, AnyVecLayout, FastHasher>,
    /// Adjacency cache for `(from, added_component) → to`. [`FastHasher`] —
    /// this is a hot lookup, the keys are well-distributed, and the entries
    /// only affect insert performance, not the determinism digest.
    add_edges: HashMap<(ArchetypeId, TypeStableId), ArchetypeId, FastHasher>,
    /// Sister cache for `(from, removed_component) → to`. Populated by
    /// `dest_with_removed` (used by `World::remove<T>`); reads route
    /// through the same hashmap on hot paths.
    remove_edges: HashMap<(ArchetypeId, TypeStableId), ArchetypeId, FastHasher>,
}

impl ArchetypeIndex {
    /// Builds an empty index with the canonical empty archetype already
    /// allocated as `ArchetypeId::EMPTY`.
    pub(crate) fn new() -> Self {
        let mut by_signature: HashMap<ArchetypeSignature, ArchetypeId, DeterministicHasher> =
            HashMap::with_hasher(DeterministicHasher::new());
        let empty_sig = ArchetypeSignature::empty();
        by_signature.insert(empty_sig.clone(), ArchetypeId::EMPTY);
        Self {
            archetypes: vec![Archetype::new(ArchetypeId::EMPTY, empty_sig, Vec::new())],
            by_signature,
            layouts: HashMap::with_hasher(FastHasher::new()),
            add_edges: HashMap::with_hasher(FastHasher::new()),
            remove_edges: HashMap::with_hasher(FastHasher::new()),
        }
    }

    /// Number of archetypes currently allocated.
    pub(crate) fn archetype_count(&self) -> usize {
        self.archetypes.len()
    }

    /// Registers the layout for component type `T`. Called by the world the
    /// first time it encounters `T` so subsequent archetypes that include
    /// `T` reuse the same drop function and stride.
    pub(crate) fn register_layout(&mut self, id: TypeStableId, layout: AnyVecLayout) {
        // Insert only if not already known. The first registration wins; a
        // second insert would silently rebind the drop_fn, which would be a
        // bug.
        if !self.layouts.contains_key(&id) {
            self.layouts.insert(id, layout);
        }
    }

    /// Look up an archetype by id.
    pub(crate) fn get(&self, id: ArchetypeId) -> &Archetype {
        &self.archetypes[id.0 as usize]
    }

    /// Mutable lookup.
    pub(crate) fn get_mut(&mut self, id: ArchetypeId) -> &mut Archetype {
        &mut self.archetypes[id.0 as usize]
    }

    /// Borrows two archetypes mutably at once. Panics if `a == b`.
    pub(crate) fn pair_mut(
        &mut self,
        a: ArchetypeId,
        b: ArchetypeId,
    ) -> (&mut Archetype, &mut Archetype) {
        assert_ne!(a, b, "pair_mut called with equal archetypes");
        let ai = a.0 as usize;
        let bi = b.0 as usize;
        if ai < bi {
            let (left, right) = self.archetypes.split_at_mut(bi);
            (&mut left[ai], &mut right[0])
        } else {
            let (left, right) = self.archetypes.split_at_mut(ai);
            (&mut right[0], &mut left[bi])
        }
    }

    /// Returns the id of the archetype with the given signature, allocating
    /// a fresh one on miss. The new archetype's columns are constructed from
    /// the cached per-component layouts.
    pub(crate) fn intern(&mut self, signature: ArchetypeSignature) -> ArchetypeId {
        if let Some(id) = self.by_signature.get(&signature) {
            return *id;
        }
        let id = ArchetypeId(self.archetypes.len() as u32);
        let layouts: Vec<AnyVecLayout> = signature
            .as_slice()
            .iter()
            .map(|sid| {
                *self.layouts.get(sid).unwrap_or_else(|| {
                    panic!(
                        "intern: no AnyVecLayout cached for {sid} — register_layout must be \
                         called before introducing the component to an archetype"
                    )
                })
            })
            .collect();
        let archetype = Archetype::new(id, signature.clone(), layouts);
        self.archetypes.push(archetype);
        self.by_signature.insert(signature, id);
        id
    }

    /// Returns the destination archetype for `(from, added)`. Uses the
    /// adjacency cache on hit; on miss, interns the resulting signature and
    /// records the edge.
    pub(crate) fn dest_with_added(
        &mut self,
        from: ArchetypeId,
        added: TypeStableId,
    ) -> ArchetypeId {
        if let Some(&id) = self.add_edges.get(&(from, added)) {
            return id;
        }
        let new_signature = self.archetypes[from.0 as usize].signature.with_added(added);
        let id = self.intern(new_signature);
        self.add_edges.insert((from, added), id);
        id
    }

    /// Returns the destination archetype for `(from, removed)`. Uses the
    /// adjacency cache on hit; on miss, interns the resulting signature and
    /// records the edge.
    pub(crate) fn dest_with_removed(
        &mut self,
        from: ArchetypeId,
        removed: TypeStableId,
    ) -> ArchetypeId {
        if let Some(&id) = self.remove_edges.get(&(from, removed)) {
            return id;
        }
        let new_signature = self.archetypes[from.0 as usize]
            .signature
            .with_removed(removed);
        let id = self.intern(new_signature);
        self.remove_edges.insert((from, removed), id);
        id
    }
}

// A small assertion to keep the size of the row index pinned — entity slots
// reference rows as `u32`, so a single archetype is capped at 4 G rows. That
// is far above the 1 M-entity Phase 3 milestone (the milestone uses one
// archetype with 1 M rows).
const _: () = assert!(std::mem::size_of::<ArchetypeRow>() == 4);

// `NonNull<u8>` is what the early implementation used; switched to a raw
// pointer to keep ZST handling simpler. Re-export the type so future code
// can ask for "not null" via the conversion if needed.
#[allow(dead_code)]
type _AnyVecPtr = NonNull<u8>;
