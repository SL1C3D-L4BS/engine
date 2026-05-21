//! Remembered set — old→young cross-gen pointers (ADR-035 follow-up).
//!
//! Single-gen `Heap` in PR 2 has no cross-gen edges; the remembered set
//! is empty by construction. The type is reserved so the generational
//! variant can drop in without touching call sites.

use crate::gc::GcHandle;

/// Set of `old gen → young gen` handles, populated by the write barrier
/// and consumed by minor collections as part of the root set.
#[derive(Debug, Default)]
pub struct RememberedSet {
    entries: Vec<GcHandle>,
}

impl RememberedSet {
    /// An empty set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a cross-gen edge from `source`.
    pub fn record(&mut self, source: GcHandle) {
        self.entries.push(source);
    }

    /// Borrows the set's contents.
    pub fn entries(&self) -> &[GcHandle] {
        &self.entries
    }

    /// Clears the set after a minor collection.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}
