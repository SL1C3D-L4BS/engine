//! Old-generation incremental tri-color heap (roadmap for ADR-035 follow-up).

/// Incremental tri-color mark-and-sweep over the old generation.
/// PR 2 keeps the single-generation `Heap` as the live implementation;
/// this type holds the place for the future split.
#[derive(Debug, Default)]
pub struct OldGen;

impl OldGen {
    /// Constructs an empty old gen.
    pub fn new() -> Self {
        Self
    }
}
