//! Write-barrier hooks.
//!
//! The Dijkstra-style barrier the generational follow-up needs is invoked
//! from `vm/dispatch.rs` on opcodes that store a handle into a heap
//! object. PR 2's single-gen heap collapses the barrier to a no-op
//! (every object lives in the same generation, so cross-gen edges are
//! impossible); keeping the call site lets the implementation evolve
//! without touching the dispatch loop again.

use crate::gc::GcHandle;

/// Records that `source` now holds a reference to `target`. Currently
/// a no-op (single-gen heap); reserved for the generational variant.
#[inline(always)]
pub fn write_barrier_hook(_source: GcHandle, _target: GcHandle) {
    // PR 2: single-generation heap — no cross-gen pointers to record.
}
