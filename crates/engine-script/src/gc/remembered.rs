//! Remembered set — old→young cross-gen pointers (ADR-059).
//!
//! Generational GC pillar 3 of 4. The card-marking strategy: one byte
//! per `CARD_SIZE` region (see `old_gen::CARD_SIZE`). When the write
//! barrier (ADR-059 §4) records a store of a nursery handle into an
//! old-gen object at slot `S`, the card byte at `S / CARD_SIZE` is
//! marked dirty. A minor collection scans every dirty card's slots for
//! nursery references — never any old-gen storage outside a dirty card.
//!
//! Card marking is one byte per card so the typical workload (zero
//! cards dirty) is a single u8 read per minor collection — the
//! literature's classic generational win.

use crate::gc::GcHandle;
use crate::gc::old_gen::CARD_SIZE;

/// Set of dirty cards plus a discrete list of cross-gen edges. The
/// discrete list is consulted by minor collections to traverse old→young
/// edges without walking the entire dirty card; the card bitmap is the
/// fast-path "no edges to look at" signal.
#[derive(Debug, Default)]
pub struct RememberedSet {
    /// One byte per card. Index = card index. Nonzero = dirty.
    cards: Vec<u8>,
    /// The actual (old_handle, target_young_handle) edges, recorded by
    /// the write barrier and consumed by the minor collector as roots.
    /// `old_handle` is kept around so the minor collector can scan the
    /// object for *all* its nursery refs (a single dirty card may
    /// hold many overlapping edges).
    entries: Vec<GcHandle>,
}

impl RememberedSet {
    /// An empty set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Resize the card bitmap to cover at least `slots` worth of old gen.
    pub fn resize_for(&mut self, slot_count: u32) {
        let needed = slot_count.div_ceil(CARD_SIZE);
        if needed > self.cards.len() as u32 {
            self.cards.resize(needed as usize, 0);
        }
    }

    /// Mark the card containing `old_slot_idx` as dirty.
    pub fn dirty_card_for_slot(&mut self, old_slot_idx: u32) {
        let card = old_slot_idx / CARD_SIZE;
        if (card as usize) < self.cards.len() {
            self.cards[card as usize] = 1;
        }
    }

    /// Records a cross-gen edge from an old-gen handle pointing to a
    /// young-gen target. Consumed by the minor collector as a root.
    pub fn record(&mut self, source: GcHandle) {
        self.entries.push(source);
    }

    /// Number of dirty cards.
    pub fn dirty_count(&self) -> u32 {
        self.cards.iter().filter(|&&b| b != 0).count() as u32
    }

    /// Borrow the (source) entries — the set of old-gen handles whose
    /// scanning the minor collector must perform.
    pub fn entries(&self) -> &[GcHandle] {
        &self.entries
    }

    /// Clear after a minor collection completes.
    pub fn clear(&mut self) {
        for b in &mut self.cards {
            *b = 0;
        }
        self.entries.clear();
    }

    /// Whether a given card is dirty.
    pub fn card_dirty(&self, card: u32) -> bool {
        self.cards.get(card as usize).copied().unwrap_or(0) != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gc::GcHandle;

    #[test]
    fn record_and_clear() {
        let mut r = RememberedSet::new();
        r.resize_for(2048);
        r.dirty_card_for_slot(100);
        r.dirty_card_for_slot(700);
        r.record(GcHandle(0x8000_0001));
        assert_eq!(r.dirty_count(), 2);
        assert_eq!(r.entries().len(), 1);
        r.clear();
        assert_eq!(r.dirty_count(), 0);
        assert_eq!(r.entries().len(), 0);
    }

    #[test]
    fn card_indexing() {
        let mut r = RememberedSet::new();
        r.resize_for(CARD_SIZE * 4);
        r.dirty_card_for_slot(0);
        r.dirty_card_for_slot(CARD_SIZE * 2);
        assert!(r.card_dirty(0));
        assert!(r.card_dirty(2));
        assert!(!r.card_dirty(1));
        assert!(!r.card_dirty(3));
    }
}
