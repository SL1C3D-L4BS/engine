//! Dijkstra-style generational write barrier (ADR-059 §4).
//!
//! The barrier is invoked from `vm/dispatch.rs` on opcodes that *store*
//! a handle into a heap object (the aggregate-mutating opcodes from
//! ADR-060: `ArraySet`, `MapSet`, `StructSet`, plus the closure /
//! struct constructors when they capture nursery values). The barrier
//! records the cross-gen edge into the remembered set so the next minor
//! collection treats `source` as a root.
//!
//! Discipline: only fires when `source` lives in old gen and `target`
//! lives in young gen. Same-gen stores are no-ops at the cost of a
//! single high-bit check. The heap façade encodes the generation in
//! the high bit of `GcHandle.0` — see `gc::mod`.

use crate::gc::{GcHandle, OLD_GEN_BIT};

/// True if `h` points into the old generation.
#[inline(always)]
#[allow(dead_code)]
pub fn is_old(h: GcHandle) -> bool {
    (h.0 & OLD_GEN_BIT) != 0
}

/// True if `h` points into the young generation.
#[inline(always)]
#[allow(dead_code)]
pub fn is_young(h: GcHandle) -> bool {
    (h.0 & OLD_GEN_BIT) == 0
}

/// The write barrier hook. Called by `vm::dispatch` on every store that
/// could create an old→young edge. The implementation here is the
/// signature only — the actual recording requires a `&mut Heap`, so the
/// dispatch loop calls `Heap::write_barrier(source, target)` instead of
/// this free function. This function exists for documentation and for
/// any future code path that has the source/target handles but no
/// `Heap` borrow (none today).
///
/// Documented for completeness; the runtime path is via
/// [`crate::gc::Heap::write_barrier`].
#[inline(always)]
#[allow(dead_code)]
pub fn would_fire(source: GcHandle, target: GcHandle) -> bool {
    is_old(source) && is_young(target)
}

/// Back-compat shim for the original single-generation API. The
/// generational façade calls `Heap::write_barrier` directly; this
/// function is kept so any older code site that imported the symbol
/// continues to compile (no-op).
#[inline(always)]
pub fn write_barrier_hook(_source: GcHandle, _target: GcHandle) {
    // Real recording happens in `Heap::write_barrier`. This shim is the
    // pre-generational call-site placeholder.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classification() {
        let young = GcHandle(0x0000_00FF);
        let old = GcHandle(0x8000_00FF);
        assert!(is_young(young));
        assert!(is_old(old));
        assert!(!is_old(young));
        assert!(!is_young(old));
    }

    #[test]
    fn would_fire_only_for_old_to_young() {
        let young_a = GcHandle(0x0000_0001);
        let young_b = GcHandle(0x0000_0002);
        let old_a = GcHandle(0x8000_0001);
        let old_b = GcHandle(0x8000_0002);
        assert!(!would_fire(young_a, young_b));
        assert!(!would_fire(young_a, old_b));
        assert!(!would_fire(old_a, old_b));
        assert!(would_fire(old_a, young_b));
    }
}
