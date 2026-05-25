//! Young-generation bump-allocator nursery (ADR-059).
//!
//! Generational GC pillar 1 of 4. Objects are born in the nursery; the
//! first survival keeps them in the nursery (re-marked); the second
//! survival promotes them to the old generation. A bump allocator over a
//! 4 MiB pre-reserved region is the canonical layout (Dragon Book §8.6;
//! Jones/Lins §8); the implementation here keeps the algorithm without
//! the page-aligned VM machinery — the Vec backing the slots is the
//! engine's allocator's responsibility, and bump-allocation is
//! `Vec::push` until the slot count hits the soft cap.
//!
//! See ADR-059 for the generational contract and ADR-035 for the
//! original VM/GC architecture. Promotion policy: an object is promoted
//! on its second minor collection.

use crate::gc::{Color, Obj};

/// Soft cap on nursery slot count. Once exceeded, the next minor
/// collection promotes survivors aggressively. The value matches the
/// ~4 MiB / 64 B per slot heuristic the design specifies. The slot
/// layout below is wider than 64 B (an Obj enum can hold a Vec) so the
/// real bytes used vary; the slot count is the steering metric.
pub const NURSERY_SOFT_CAP: u32 = 65_536;

/// Survival counter at which a nursery object is promoted to old gen.
/// Two means "survived two minor collections."
pub const PROMOTION_AGE: u8 = 2;

/// One nursery slot. The slot tracks the object, its mark color, and its
/// survival counter (for promotion). A swept slot's `obj` is `None`; the
/// slot index is recycled by the next allocation.
#[derive(Debug)]
pub struct NurserySlot {
    /// The object payload. `None` after sweep.
    pub obj: Option<Obj>,
    /// Mark color for the current collection cycle.
    pub color: Color,
    /// Number of minor collections this object has survived.
    pub age: u8,
}

/// The young generation. Slots are pushed; sweep frees slots back to a
/// free list; promotion drains an object out and returns it to the
/// caller (the heap façade puts it into the old generation).
#[derive(Debug, Default)]
pub struct Nursery {
    slots: Vec<NurserySlot>,
    free_list: Vec<u32>,
    allocations: u64,
}

impl Nursery {
    /// Constructs an empty nursery.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of live nursery slots.
    pub fn live(&self) -> u32 {
        self.slots.iter().filter(|s| s.obj.is_some()).count() as u32
    }

    /// Number of slots ever allocated (cumulative).
    pub fn total_allocations(&self) -> u64 {
        self.allocations
    }

    /// Whether the nursery has crossed its soft cap.
    pub fn over_cap(&self) -> bool {
        self.live() >= NURSERY_SOFT_CAP
    }

    /// Allocates a slot for `obj`. Returns the nursery-local index.
    pub fn alloc(&mut self, obj: Obj) -> u32 {
        self.allocations += 1;
        if let Some(idx) = self.free_list.pop() {
            self.slots[idx as usize] = NurserySlot {
                obj: Some(obj),
                color: Color::White,
                age: 0,
            };
            idx
        } else {
            let idx = self.slots.len() as u32;
            self.slots.push(NurserySlot {
                obj: Some(obj),
                color: Color::White,
                age: 0,
            });
            idx
        }
    }

    /// Borrows a slot by nursery index.
    pub fn get(&self, idx: u32) -> Option<&Obj> {
        self.slots.get(idx as usize).and_then(|s| s.obj.as_ref())
    }

    /// Mutably borrows a slot by nursery index.
    pub fn get_mut(&mut self, idx: u32) -> Option<&mut Obj> {
        self.slots
            .get_mut(idx as usize)
            .and_then(|s| s.obj.as_mut())
    }

    /// Whiten every live slot. Called at the start of a minor collection.
    pub fn whiten(&mut self) {
        for slot in &mut self.slots {
            if slot.obj.is_some() {
                slot.color = Color::White;
            }
        }
    }

    /// Mark a slot black. Returns true if the slot transitioned from
    /// non-black to black (so the caller's worklist can avoid double-
    /// scanning).
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

    /// Borrow a slot's color for traversal.
    pub fn color(&self, idx: u32) -> Color {
        self.slots
            .get(idx as usize)
            .map(|s| s.color)
            .unwrap_or(Color::White)
    }

    /// Borrow a slot's age.
    pub fn age(&self, idx: u32) -> u8 {
        self.slots.get(idx as usize).map(|s| s.age).unwrap_or(0)
    }

    /// Sweep white (unreached) slots, returning the count freed.
    /// Black slots are aged (their survival counter ticks up) and stay.
    pub fn sweep(&mut self) -> u64 {
        let mut freed = 0u64;
        for (idx, slot) in self.slots.iter_mut().enumerate() {
            if slot.obj.is_some() && slot.color == Color::White {
                slot.obj = None;
                self.free_list.push(idx as u32);
                freed += 1;
            } else if slot.obj.is_some() {
                slot.age = slot.age.saturating_add(1);
            }
        }
        freed
    }

    /// Drain (and free) the slot at `idx`, returning the object so the
    /// caller can promote it to the old generation. Called by the heap
    /// façade during minor collection for objects whose age reaches
    /// `PROMOTION_AGE`.
    pub fn promote(&mut self, idx: u32) -> Option<Obj> {
        let slot = self.slots.get_mut(idx as usize)?;
        let obj = slot.obj.take()?;
        slot.color = Color::White;
        slot.age = 0;
        self.free_list.push(idx);
        Some(obj)
    }

    /// Enumerate live nursery indices.
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
    fn alloc_and_get() {
        let mut n = Nursery::new();
        let idx = n.alloc(Obj::Array(vec![Value::Int(1)]));
        assert!(n.get(idx).is_some());
        assert_eq!(n.live(), 1);
    }

    #[test]
    fn sweep_unmarked() {
        let mut n = Nursery::new();
        let _ = n.alloc(Obj::Array(vec![]));
        let kept = n.alloc(Obj::Array(vec![]));
        n.whiten();
        n.mark_black(kept);
        let freed = n.sweep();
        assert_eq!(freed, 1);
        assert_eq!(n.live(), 1);
        assert!(n.get(kept).is_some());
    }

    #[test]
    fn aging_then_promotion() {
        let mut n = Nursery::new();
        let idx = n.alloc(Obj::Array(vec![Value::Int(7)]));
        // Survive collection 1.
        n.whiten();
        n.mark_black(idx);
        n.sweep();
        assert_eq!(n.age(idx), 1);
        // Survive collection 2 — eligible for promotion now.
        n.whiten();
        n.mark_black(idx);
        n.sweep();
        assert_eq!(n.age(idx), PROMOTION_AGE);
        let promoted = n.promote(idx);
        assert!(matches!(promoted, Some(Obj::Array(_))));
        assert!(n.get(idx).is_none());
    }
}
