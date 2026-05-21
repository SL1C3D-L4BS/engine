//! Young-generation nursery (roadmap for ADR-035 follow-up).
//!
//! PR 2 ships the single-generation `Heap` in `mod.rs`. This module
//! is a placeholder for the 4 MiB bump-allocator nursery the design
//! calls for; the public type exists so callers in PR 3 can take a
//! `Nursery` parameter and only the implementation has to evolve.

/// 4 MiB bump-allocator young generation. Empty in PR 2; the type is
/// reserved so the API surface is stable.
#[derive(Debug, Default)]
pub struct Nursery;

impl Nursery {
    /// Constructs an empty nursery.
    pub fn new() -> Self {
        Self
    }
}
