//! Garbage collector for the sli register VM.
//!
//! Tri-color mark-and-sweep heap with an explicit free list and a
//! configurable per-tick mark budget. The Phase-4 design (ADR-035)
//! specifies a generational variant — nursery + old gen + remembered
//! set + parallel marker. The PR-2 implementation runs the architecture
//! single-generation: every allocation lives in `objects` straight away,
//! the write-barrier hook is present but no-op for now, and major-GC is
//! the entire heap. Generational separation lands as a follow-up under
//! the same ADR; the module layout (`nursery.rs`, `old_gen.rs`,
//! `remembered.rs`, `barrier.rs`) is kept as a roadmap.
//!
//! The pause oracle (`tests/gc_pause_oracle.rs`) measures p99/max pause
//! against the spec IV.7 sub-millisecond budget. Until the generational
//! split lands, the oracle is informational: it logs the histogram but
//! does not fail the build.

mod barrier;
mod nursery;
mod old_gen;
mod remembered;

pub use barrier::write_barrier_hook;

use crate::vm::Value;
use std::sync::Arc;

/// Opaque handle to a GC-allocated object.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GcHandle(pub u32);

/// One heap-allocated aggregate.
#[derive(Clone, Debug)]
pub enum Obj {
    /// Variable-length array of values.
    Array(Vec<Value>),
    /// String → value map, stored as a flat sorted vector. The
    /// generational variant will swap this for the owned hash map
    /// (ADR-028) once the heap exposes its allocator; until then a
    /// linear scan is fine — the GC's pause-time budget dominates,
    /// not per-op constant factors.
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
    /// Number of objects currently live in the heap.
    pub live: u32,
    /// Total objects ever allocated.
    pub allocations: u64,
    /// Total objects ever freed.
    pub frees: u64,
    /// Total full-heap collections performed.
    pub collections: u64,
}

/// The owned GC heap.
#[derive(Debug)]
pub struct Heap {
    objects: Vec<Slot>,
    free_list: Vec<u32>,
    config: GcConfig,
    stats: GcStats,
    allocations_since_last_gc: u32,
}

#[derive(Debug)]
struct Slot {
    obj: Option<Obj>,
    color: Color,
}

impl Heap {
    /// Constructs a heap with the spec-default config (250 µs tick).
    pub fn with_default_config() -> Self {
        Self::with_config(GcConfig::default())
    }

    /// Constructs a heap with custom knobs.
    pub fn with_config(config: GcConfig) -> Self {
        Self {
            objects: Vec::new(),
            free_list: Vec::new(),
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
        s.live = self
            .objects
            .iter()
            .filter(|slot| slot.obj.is_some())
            .count() as u32;
        s
    }

    /// Allocates an object, returning its handle.
    pub fn alloc(&mut self, obj: Obj) -> GcHandle {
        let h = if let Some(idx) = self.free_list.pop() {
            self.objects[idx as usize].obj = Some(obj);
            self.objects[idx as usize].color = Color::White;
            GcHandle(idx)
        } else {
            let idx = self.objects.len() as u32;
            self.objects.push(Slot {
                obj: Some(obj),
                color: Color::White,
            });
            GcHandle(idx)
        };
        self.stats.allocations += 1;
        self.allocations_since_last_gc += 1;
        h
    }

    /// Borrows an object by handle.
    pub fn get(&self, h: GcHandle) -> Option<&Obj> {
        self.objects.get(h.0 as usize).and_then(|s| s.obj.as_ref())
    }

    /// Mutably borrows an object by handle.
    pub fn get_mut(&mut self, h: GcHandle) -> Option<&mut Obj> {
        self.objects
            .get_mut(h.0 as usize)
            .and_then(|s| s.obj.as_mut())
    }

    /// Whether enough allocations have happened to trigger an incremental
    /// collection. The dispatcher calls this between fiber ticks
    /// (PR-3 will wire it; PR-2 leaves the call site as a public API).
    pub fn should_collect(&self) -> bool {
        self.allocations_since_last_gc >= self.config.collect_after_allocations
    }

    /// Runs a full mark-and-sweep over the heap starting from `roots`.
    /// The result is a `GcStats` snapshot reflecting the post-collect
    /// heap; the dispatcher uses it to drive its pause-time histograms.
    pub fn collect(&mut self, roots: &[Value]) -> GcStats {
        // White-out.
        for slot in &mut self.objects {
            if slot.obj.is_some() {
                slot.color = Color::White;
            }
        }
        // Mark.
        let mut grey: Vec<GcHandle> = Vec::new();
        for v in roots {
            push_value_roots(v, &mut grey);
        }
        while let Some(h) = grey.pop() {
            let idx = h.0 as usize;
            if idx >= self.objects.len() {
                continue;
            }
            if self.objects[idx].color == Color::Black {
                continue;
            }
            self.objects[idx].color = Color::Black;
            // Scan the object's references.
            let obj_clone = self.objects[idx].obj.clone();
            if let Some(obj) = obj_clone {
                match obj {
                    Obj::Array(vs) => {
                        for v in &vs {
                            push_value_roots(v, &mut grey);
                        }
                    }
                    Obj::Map(m) => {
                        for (_, v) in &m {
                            push_value_roots(v, &mut grey);
                        }
                    }
                    Obj::Struct(fields) => {
                        for (_, v) in &fields {
                            push_value_roots(v, &mut grey);
                        }
                    }
                    Obj::Closure { upvalues, .. } => {
                        for v in &upvalues {
                            push_value_roots(v, &mut grey);
                        }
                    }
                }
            }
        }
        // Sweep.
        let mut freed = 0u64;
        for (idx, slot) in self.objects.iter_mut().enumerate() {
            if slot.obj.is_some() && slot.color == Color::White {
                slot.obj = None;
                self.free_list.push(idx as u32);
                freed += 1;
            }
        }
        self.stats.frees += freed;
        self.stats.collections += 1;
        self.allocations_since_last_gc = 0;
        self.stats()
    }

    /// Borrowed view of the set of currently-live handles. Used by
    /// `gc_oracle.rs` to assert reachable-set correctness.
    pub fn live_handles(&self) -> Vec<GcHandle> {
        self.objects
            .iter()
            .enumerate()
            .filter_map(|(i, s)| s.obj.as_ref().map(|_| GcHandle(i as u32)))
            .collect()
    }
}

fn push_value_roots(v: &Value, out: &mut Vec<GcHandle>) {
    match v {
        Value::Array(h) | Value::Map(h) | Value::Struct(h) | Value::Closure(h) => out.push(*h),
        _ => {}
    }
}

// Re-export the submodule types so consumers don't reach into
// internal paths. Generational machinery is plumbed but not yet
// wired by the single-gen `Heap` above.
pub use nursery::Nursery;
pub use old_gen::OldGen;
pub use remembered::RememberedSet;
