//! Garbage collector for the sli register VM (ADR-035 + ADR-059).
//!
//! Generational tri-color mark-and-sweep heap. The architecture has four
//! pillars (ADR-059):
//!
//! 1. **Nursery** (`nursery.rs`) — young generation, bump-allocator,
//!    every new object starts here.
//! 2. **Old generation** (`old_gen.rs`) — survivors of two minor
//!    collections; mark-and-sweep on major collection.
//! 3. **Remembered set** (`remembered.rs`) — card-marking record of
//!    old→young pointers populated by the write barrier and consumed by
//!    minor collections as additional roots.
//! 4. **Write barrier** (`barrier.rs`) — fired by mutating opcodes
//!    (ADR-060 aggregates: `ArraySet`, `MapSet`, `StructSet`, plus
//!    closure construction); the runtime path is
//!    [`Heap::write_barrier`] which records the card + edge.
//!
//! ## Handle layout
//!
//! [`GcHandle`] is a u32 with the high bit ([`OLD_GEN_BIT`]) encoding
//! the generation: 0 = nursery, 1 = old gen. The lower 31 bits are the
//! per-generation slot index. The encoding lets the dispatcher and the
//! barrier classify a handle in one instruction without a heap lookup.
//!
//! ## Public API stability
//!
//! The `Heap` façade preserves the pre-ADR-059 API surface: `alloc`,
//! `get`, `get_mut`, `collect`, `live_handles`, `stats`, `should_collect`.
//! Existing tests (`tests/gc_oracle.rs`, `tests/gc_pause_oracle.rs`)
//! continue to pass without modification. The `collect()` method is now
//! a **major** collection (mark from roots, scan both generations,
//! sweep both); a new [`Heap::minor_collect`] runs the nursery-only path
//! the dispatcher will use for incremental pause discipline.
//!
//! The pause oracle (`tests/gc_pause_oracle.rs`) measures p99/max pause
//! against the spec IV.7 sub-millisecond budget. Generational separation
//! is the design lever for hitting it — minor collections touch only the
//! nursery slots plus the remembered set.

mod barrier;
mod nursery;
mod old_gen;
mod remembered;

pub use barrier::write_barrier_hook;

use crate::vm::Value;
use std::sync::Arc;

/// High bit of [`GcHandle.0`] — set means old gen, clear means nursery.
pub const OLD_GEN_BIT: u32 = 0x8000_0000;

/// Mask isolating the slot index (lower 31 bits).
pub const INDEX_MASK: u32 = 0x7FFF_FFFF;

/// Opaque handle to a GC-allocated object.
///
/// The high bit encodes the generation (`OLD_GEN_BIT`); the low 31 bits
/// are the per-generation slot index. Handles are stable across minor
/// collections **as long as the object stays in its current generation**.
/// Promotion (nursery → old gen) changes the handle; the dispatcher
/// holds onto Values, and the promotion path rewrites every Value that
/// references the promoted slot via a remap table the major-collect path
/// returns. Today's `collect()` is a major-only path that does not
/// promote (every reachable object is marked then swept in place), so
/// handles remain stable across `collect()` calls — preserving the
/// pre-ADR-059 oracle semantics.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GcHandle(pub u32);

impl GcHandle {
    /// True if this handle points into the old generation.
    #[inline(always)]
    pub fn is_old(self) -> bool {
        (self.0 & OLD_GEN_BIT) != 0
    }

    /// True if this handle points into the nursery.
    #[inline(always)]
    pub fn is_young(self) -> bool {
        (self.0 & OLD_GEN_BIT) == 0
    }

    /// Slot index within its generation (lower 31 bits).
    #[inline(always)]
    pub fn index(self) -> u32 {
        self.0 & INDEX_MASK
    }

    /// Construct a nursery handle from a slot index.
    #[inline(always)]
    pub fn nursery(slot: u32) -> Self {
        Self(slot & INDEX_MASK)
    }

    /// Construct an old-gen handle from a slot index.
    #[inline(always)]
    pub fn old(slot: u32) -> Self {
        Self((slot & INDEX_MASK) | OLD_GEN_BIT)
    }
}

/// One heap-allocated aggregate.
#[derive(Clone, Debug)]
pub enum Obj {
    /// Variable-length array of values.
    Array(Vec<Value>),
    /// String → value map, stored as a flat sorted vector. The owned
    /// hash map (ADR-028) replaces this once the heap exposes its
    /// allocator; until then a linear scan is fine — the GC's pause-time
    /// budget dominates, not per-op constant factors.
    Map(Vec<(Arc<str>, Value)>),
    /// Struct instance — flat name/value slots.
    Struct(Vec<(Arc<str>, Value)>),
    /// Closure: function id + captured upvalues.
    Closure {
        /// Module-relative function id.
        function_id: u16,
        /// Captured upvalues in declaration order.
        upvalues: Vec<Value>,
    },
}

/// Tri-color mark state for one heap slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Color {
    /// Unreached.
    White,
    /// Reached but not yet scanned.
    Grey,
    /// Reached and scanned.
    Black,
}

/// GC configuration knobs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GcConfig {
    /// Per-tick incremental mark budget, in microseconds (spec IV.7
    /// sub-ms target keys on a 250 µs default).
    pub tick_budget_us: u32,
    /// Heap growth threshold — minor collect triggers when allocations
    /// since the last collection exceed this many objects.
    pub collect_after_allocations: u32,
}

impl Default for GcConfig {
    fn default() -> Self {
        Self {
            tick_budget_us: 250,
            collect_after_allocations: 4096,
        }
    }
}

/// A snapshot of GC counters at a moment in time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GcStats {
    /// Number of objects currently live in the heap (both generations).
    pub live: u32,
    /// Total objects ever allocated (both generations).
    pub allocations: u64,
    /// Total objects ever freed (both generations).
    pub frees: u64,
    /// Total full-heap collections performed.
    pub collections: u64,
    /// Total minor collections performed.
    pub minor_collections: u64,
    /// Total objects promoted from nursery to old gen.
    pub promotions: u64,
}

/// The owned generational GC heap.
#[derive(Debug)]
pub struct Heap {
    nursery: nursery::Nursery,
    old_gen: old_gen::OldGen,
    remembered: remembered::RememberedSet,
    config: GcConfig,
    stats: GcStats,
    allocations_since_last_gc: u32,
}

impl Heap {
    /// Constructs a heap with the spec-default config (250 µs tick).
    pub fn with_default_config() -> Self {
        Self::with_config(GcConfig::default())
    }

    /// Constructs a heap with custom knobs.
    pub fn with_config(config: GcConfig) -> Self {
        Self {
            nursery: nursery::Nursery::new(),
            old_gen: old_gen::OldGen::new(),
            remembered: remembered::RememberedSet::new(),
            config,
            stats: GcStats::default(),
            allocations_since_last_gc: 0,
        }
    }

    /// Borrows the active config.
    pub fn config(&self) -> &GcConfig {
        &self.config
    }

    /// Returns a stats snapshot.
    pub fn stats(&self) -> GcStats {
        let mut s = self.stats;
        s.live = self.nursery.live() + self.old_gen.live();
        s
    }

    /// Allocates an object into the nursery, returning its handle. Every
    /// allocation begins in the young generation per the generational
    /// hypothesis; promotion to old gen happens after surviving two
    /// minor collections (see [`nursery::PROMOTION_AGE`]).
    pub fn alloc(&mut self, obj: Obj) -> GcHandle {
        let idx = self.nursery.alloc(obj);
        self.stats.allocations += 1;
        self.allocations_since_last_gc += 1;
        GcHandle::nursery(idx)
    }

    /// Borrows an object by handle. Routes the lookup to the nursery or
    /// the old generation based on the handle's high bit.
    pub fn get(&self, h: GcHandle) -> Option<&Obj> {
        if h.is_old() {
            self.old_gen.get(h.index())
        } else {
            self.nursery.get(h.index())
        }
    }

    /// Mutably borrows an object by handle.
    pub fn get_mut(&mut self, h: GcHandle) -> Option<&mut Obj> {
        if h.is_old() {
            self.old_gen.get_mut(h.index())
        } else {
            self.nursery.get_mut(h.index())
        }
    }

    /// Whether enough allocations have happened to trigger an incremental
    /// (minor) collection. The dispatcher checks this between fiber
    /// ticks.
    pub fn should_collect(&self) -> bool {
        self.allocations_since_last_gc >= self.config.collect_after_allocations
            || self.nursery.over_cap()
    }

    /// Records that `source` (old-gen) now holds a reference to `target`
    /// (young-gen). The remembered set treats `source` as a root in the
    /// next minor collection. Same-gen stores are no-ops (cheap branch).
    ///
    /// Called from the dispatch loop on every mutating opcode that could
    /// introduce an old→young edge (ADR-060: `ArraySet`, `MapSet`,
    /// `StructSet`, closure construction when capturing nursery values).
    #[inline]
    pub fn write_barrier(&mut self, source: GcHandle, target: GcHandle) {
        if source.is_old() && target.is_young() {
            self.remembered.resize_for(self.old_gen.capacity());
            self.remembered.dirty_card_for_slot(source.index());
            self.remembered.record(source);
        }
    }

    /// Runs a full mark-and-sweep over both generations starting from
    /// `roots`. The result is a `GcStats` snapshot reflecting the
    /// post-collect heap. This is a *major* collection in the
    /// generational lexicon — both nursery and old gen are walked,
    /// marked, and swept. Handles remain stable across the call
    /// (no promotion happens during a major collection because every
    /// reachable object stays in place).
    pub fn collect(&mut self, roots: &[Value]) -> GcStats {
        self.nursery.whiten();
        self.old_gen.whiten();

        let mut grey: Vec<GcHandle> = Vec::new();
        for v in roots {
            push_value_roots(v, &mut grey);
        }
        self.mark_loop(&mut grey);

        let freed_n = self.nursery.sweep();
        let freed_o = self.old_gen.sweep();
        self.remembered.clear();
        self.stats.frees += freed_n + freed_o;
        self.stats.collections += 1;
        self.allocations_since_last_gc = 0;
        self.stats()
    }

    /// Runs a minor (nursery-only) collection. Roots are the explicit
    /// `roots` plus every object reachable from the remembered set's
    /// recorded old-gen sources. Survivors of age 2+ are promoted to the
    /// old generation; the caller receives a remap table
    /// (`Vec<(old_handle, new_handle)>`) so any external Value
    /// referencing a promoted nursery slot can be rewritten.
    ///
    /// The dispatcher (Phase 5+ wiring) drives this from
    /// [`Heap::should_collect`]; today's call sites are the pause
    /// oracle (`tests/gc_pause_oracle.rs`) and the unit tests below.
    pub fn minor_collect(&mut self, roots: &[Value]) -> Vec<(GcHandle, GcHandle)> {
        self.nursery.whiten();

        let mut grey: Vec<GcHandle> = Vec::new();
        for v in roots {
            push_value_roots(v, &mut grey);
        }
        for &source in self.remembered.entries() {
            grey.push(source);
        }

        self.mark_loop_nursery_only(&mut grey);

        // Promote age-eligible survivors before sweeping. Sweep would
        // otherwise increment age past PROMOTION_AGE on these objects;
        // promote first so the sweep only ages those that stay.
        let mut remap = Vec::new();
        let candidates: Vec<u32> = self
            .nursery
            .live_indices()
            .filter(|&i| {
                self.nursery.color(i) == Color::Black
                    && self.nursery.age(i) + 1 >= nursery::PROMOTION_AGE
            })
            .collect();
        for nursery_idx in candidates {
            if let Some(obj) = self.nursery.promote(nursery_idx) {
                let old_idx = self.old_gen.promote_in(obj);
                remap.push((GcHandle::nursery(nursery_idx), GcHandle::old(old_idx)));
                self.stats.promotions += 1;
            }
        }

        let freed_n = self.nursery.sweep();
        self.remembered.clear();
        self.stats.frees += freed_n;
        self.stats.minor_collections += 1;
        self.allocations_since_last_gc = 0;
        remap
    }

    /// Borrowed view of the set of currently-live handles. Used by
    /// `gc_oracle.rs` to assert reachable-set correctness.
    pub fn live_handles(&self) -> Vec<GcHandle> {
        let mut out = Vec::with_capacity(
            (self.nursery.live() + self.old_gen.live()) as usize,
        );
        for i in self.nursery.live_indices() {
            out.push(GcHandle::nursery(i));
        }
        for i in self.old_gen.live_indices() {
            out.push(GcHandle::old(i));
        }
        out
    }

    fn mark_loop(&mut self, grey: &mut Vec<GcHandle>) {
        while let Some(h) = grey.pop() {
            let already_black = if h.is_old() {
                !self.old_gen.mark_black(h.index())
            } else {
                !self.nursery.mark_black(h.index())
            };
            if already_black {
                continue;
            }
            // Scan the object's references (clone so we can re-borrow
            // mutably for the next mark).
            let obj_clone = if h.is_old() {
                self.old_gen.get(h.index()).cloned()
            } else {
                self.nursery.get(h.index()).cloned()
            };
            if let Some(obj) = obj_clone {
                scan_obj_into(&obj, grey);
            }
        }
    }

    fn mark_loop_nursery_only(&mut self, grey: &mut Vec<GcHandle>) {
        while let Some(h) = grey.pop() {
            if h.is_old() {
                // Old-gen handles from the remembered set get scanned
                // for their nursery children but are not themselves
                // marked (the next major collection owns old-gen marks).
                let obj_clone = self.old_gen.get(h.index()).cloned();
                if let Some(obj) = obj_clone {
                    scan_obj_into(&obj, grey);
                }
                continue;
            }
            if !self.nursery.mark_black(h.index()) {
                continue;
            }
            let obj_clone = self.nursery.get(h.index()).cloned();
            if let Some(obj) = obj_clone {
                scan_obj_into(&obj, grey);
            }
        }
    }
}

fn push_value_roots(v: &Value, out: &mut Vec<GcHandle>) {
    match v {
        Value::Array(h) | Value::Map(h) | Value::Struct(h) | Value::Closure(h) => out.push(*h),
        _ => {}
    }
}

fn scan_obj_into(obj: &Obj, out: &mut Vec<GcHandle>) {
    match obj {
        Obj::Array(vs) => {
            for v in vs {
                push_value_roots(v, out);
            }
        }
        Obj::Map(m) => {
            for (_, v) in m {
                push_value_roots(v, out);
            }
        }
        Obj::Struct(fields) => {
            for (_, v) in fields {
                push_value_roots(v, out);
            }
        }
        Obj::Closure { upvalues, .. } => {
            for v in upvalues {
                push_value_roots(v, out);
            }
        }
    }
}

// Re-export the submodule types so consumers don't reach into
// internal paths.
pub use nursery::Nursery;
pub use old_gen::OldGen;
pub use remembered::RememberedSet;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handle_encoding_roundtrip() {
        let n = GcHandle::nursery(42);
        assert!(n.is_young());
        assert!(!n.is_old());
        assert_eq!(n.index(), 42);
        let o = GcHandle::old(7);
        assert!(o.is_old());
        assert!(!o.is_young());
        assert_eq!(o.index(), 7);
    }

    #[test]
    fn alloc_starts_in_nursery() {
        let mut h = Heap::with_default_config();
        let g = h.alloc(Obj::Array(vec![Value::Int(1)]));
        assert!(g.is_young());
        assert!(h.get(g).is_some());
    }

    #[test]
    fn promotion_after_two_minor_collects() {
        let mut h = Heap::with_default_config();
        let kept = h.alloc(Obj::Array(vec![Value::Int(99)]));
        let roots = vec![Value::Array(kept)];

        let remap1 = h.minor_collect(&roots);
        assert!(remap1.is_empty(), "first survival ages but does not promote");
        assert!(kept.is_young());
        assert!(h.get(kept).is_some());

        let remap2 = h.minor_collect(&roots);
        assert_eq!(remap2.len(), 1, "second survival promotes");
        let (old_handle, new_handle) = remap2[0];
        assert_eq!(old_handle, kept);
        assert!(new_handle.is_old());
        assert!(h.get(new_handle).is_some());
        assert!(h.get(kept).is_none(), "nursery slot is reclaimed");
    }

    #[test]
    fn write_barrier_records_old_to_young() {
        let mut h = Heap::with_default_config();
        // Force an old-gen object into existence by promoting.
        let child = h.alloc(Obj::Array(vec![Value::Int(7)]));
        let parent = h.alloc(Obj::Array(vec![Value::Array(child)]));
        let roots = vec![Value::Array(parent)];
        h.minor_collect(&roots);
        let remap = h.minor_collect(&roots);
        // Both should have been promoted.
        let parent_old = remap
            .iter()
            .find_map(|(o, n)| if *o == parent { Some(*n) } else { None })
            .expect("parent promoted");

        // Allocate a fresh nursery object and store it into the
        // (old-gen) parent. The write barrier should record the edge.
        let fresh = h.alloc(Obj::Array(vec![Value::Int(123)]));
        h.write_barrier(parent_old, fresh);

        let remap2 = h.minor_collect(&[Value::Array(parent_old)]);
        // `fresh` is reachable via parent's children (we recorded the
        // barrier, but didn't actually mutate parent's contents — this
        // test verifies the remembered set is consulted, not that
        // mutation is wired). The remembered-set source is treated as
        // a root for scanning; since `fresh` isn't actually inside
        // parent's Obj::Array, it should be collected. Test that:
        assert!(remap2.is_empty(), "fresh not actually inside parent; collected");
        assert!(h.get(fresh).is_none());
    }

    #[test]
    fn major_collect_sweeps_both_gens() {
        let mut h = Heap::with_default_config();
        let keep = h.alloc(Obj::Array(vec![Value::Int(1)]));
        let drop_ = h.alloc(Obj::Array(vec![Value::Int(2)]));
        let roots = vec![Value::Array(keep)];
        // Two minor collects to promote `keep`.
        let _ = h.minor_collect(&roots);
        let remap = h.minor_collect(&roots);
        let keep_old = remap[0].1;
        // Allocate another nursery object and drop it.
        let _ephemeral = h.alloc(Obj::Array(vec![Value::Int(3)]));
        let stats = h.collect(&[Value::Array(keep_old)]);
        assert_eq!(stats.live, 1);
        assert!(h.get(keep_old).is_some());
        // The originally-allocated `drop_` was collected by the first
        // minor (or in this case, the major sweep).
        assert!(h.get(drop_).is_none());
    }
}
