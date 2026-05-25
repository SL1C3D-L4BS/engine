//! Old-generation mark-and-sweep heap (ADR-059).
//!
//! Generational GC pillar 2 of 4. Once an object survives two minor
//! collections in the nursery it is promoted here. Old-gen objects are
//! marked and swept during *major* collections; minor collections walk
//! through old-gen roots only via the remembered set (ADR-059 §3),
//! never marking old-gen objects unless they hold pointers into the
//! nursery.
//!
//! The layout is a Vec of slots with a free list — the same shape as
//! the original single-generation `Heap` before generational split.
//! Card marking (the remembered set's storage) lives in the sibling
//! `remembered` module; this module only owns the object storage.

use crate::gc::{Color, Obj};

/// Bytes per card. The card-marking remembered set stores one byte per
/// card; the card covers a contiguous region of the old generation's
/// logical address space (here, slot indices). 512-byte cards are the
/// SpiderMonkey-derived sweet spot for engines this size.
pub const CARD_SIZE: u32 = 512;

/// One old-gen slot.
#[derive(Debug)]
pub struct OldSlot {
    pub obj: Option<Obj>,
    pub color: Color,
}

/// The old generation. Mark-and-sweep over a Vec; free list for slot
/// recycling. The bytes-per-card constant exists here so the remembered
/// set in the sibling module can size its card byte array consistently.
#[derive(Debug, Default)]
pub struct OldGen {
    slots: Vec<OldSlot>,
    free_list: Vec<u32>,
    allocations: u64,
}

impl OldGen {
    /// Constructs an empty old generation.
    pub fn new() -> Self {
        Self::default()
    }

    /// Live slot count.
    pub fn live(&self) -> u32 {
        self.slots.iter().filter(|s| s.obj.is_some()).count() as u32
    }

    /// Cumulative allocations.
    pub fn total_allocations(&self) -> u64 {
        self.allocations
    }

    /// Number of slots (live or free) — for card-set sizing.
    pub fn capacity(&self) -> u32 {
        self.slots.len() as u32
    }

    /// Number of cards covering the current capacity.
    pub fn card_count(&self) -> u32 {
        self.capacity().div_ceil(CARD_SIZE)
    }

    /// Card index that covers `slot_idx`.
    pub fn card_for_slot(slot_idx: u32) -> u32 {
        slot_idx / CARD_SIZE
    }

    /// Promote an object into the old generation, returning its old-gen
    /// index.
    pub fn promote_in(&mut self, obj: Obj) -> u32 {
        self.allocations += 1;
        if let Some(idx) = self.free_list.pop() {
            self.slots[idx as usize] = OldSlot {
                obj: Some(obj),
                color: Color::White,
            };
            idx
        } else {
            let idx = self.slots.len() as u32;
            self.slots.push(OldSlot {
                obj: Some(obj),
                color: Color::White,
            });
            idx
        }
    }

    /// Borrows an object.
    pub fn get(&self, idx: u32) -> Option<&Obj> {
        self.slots.get(idx as usize).and_then(|s| s.obj.as_ref())
    }

    /// Mutably borrows an object.
    pub fn get_mut(&mut self, idx: u32) -> Option<&mut Obj> {
        self.slots
            .get_mut(idx as usize)
            .and_then(|s| s.obj.as_mut())
    }

    /// Whiten every live slot. Called at the start of a major collection.
    pub fn whiten(&mut self) {
        for slot in &mut self.slots {
            if slot.obj.is_some() {
                slot.color = Color::White;
            }
        }
    }

    /// Mark a slot black; returns true if newly black.
    pub fn mark_black(&mut self, idx: u32) -> bool {
        let Some(slot) = self.slots.get_mut(idx as usize) else {
            return false;
        };
        if slot.color == Color::Black || slot.obj.is_none() {
            return false;
        }
        slot.color = Color::Black;
        true
    }

    /// Borrow a slot's color.
    pub fn color(&self, idx: u32) -> Color {
        self.slots
            .get(idx as usize)
            .map(|s| s.color)
            .unwrap_or(Color::White)
    }

    /// Sweep white slots. Returns the count freed.
    pub fn sweep(&mut self) -> u64 {
        let mut freed = 0u64;
        for (idx, slot) in self.slots.iter_mut().enumerate() {
            if slot.obj.is_some() && slot.color == Color::White {
                slot.obj = None;
                self.free_list.push(idx as u32);
                freed += 1;
            }
        }
        freed
    }

    /// Enumerate live old-gen indices.
    pub fn live_indices(&self) -> impl Iterator<Item = u32> + '_ {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(i, s)| s.obj.as_ref().map(|_| i as u32))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::Value;

    #[test]
    fn promote_and_get() {
        let mut o = OldGen::new();
        let idx = o.promote_in(Obj::Array(vec![Value::Int(42)]));
        assert!(o.get(idx).is_some());
        assert_eq!(o.live(), 1);
    }

    #[test]
    fn major_sweep_drops_white() {
        let mut o = OldGen::new();
        let dead = o.promote_in(Obj::Array(vec![]));
        let live = o.promote_in(Obj::Array(vec![]));
        o.whiten();
        o.mark_black(live);
        let freed = o.sweep();
        assert_eq!(freed, 1);
        assert!(o.get(dead).is_none());
        assert!(o.get(live).is_some());
    }

    #[test]
    fn card_indexing() {
        assert_eq!(OldGen::card_for_slot(0), 0);
        assert_eq!(OldGen::card_for_slot(CARD_SIZE - 1), 0);
        assert_eq!(OldGen::card_for_slot(CARD_SIZE), 1);
        assert_eq!(OldGen::card_for_slot(CARD_SIZE * 3 + 7), 3);
    }
}
