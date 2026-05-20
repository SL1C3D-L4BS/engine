//! Entity identifiers and the entity allocator.

use core::fmt;

/// An entity handle.
///
/// Packed as `u64 = [generation: u32 | index: u32]` (spec IV.3). The index
/// addresses a slot in component storage and is recycled when the entity dies;
/// the generation is bumped on death so a stale [`Entity`] copy no longer
/// matches the live entity occupying its index.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Entity(u64);

impl Entity {
    /// Builds an entity from an index and generation.
    pub(crate) const fn from_parts(index: u32, generation: u32) -> Self {
        Self(((generation as u64) << 32) | index as u64)
    }

    /// The storage slot index.
    #[inline]
    pub const fn index(self) -> u32 {
        self.0 as u32
    }

    /// The generation counter.
    #[inline]
    pub const fn generation(self) -> u32 {
        (self.0 >> 32) as u32
    }

    /// The raw packed `u64`.
    #[inline]
    pub const fn to_bits(self) -> u64 {
        self.0
    }
}

impl fmt::Debug for Entity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Matches the editor's `0x[index][gen]` display (spec IV.3).
        write!(f, "Entity(0x{:08x}{:08x})", self.index(), self.generation())
    }
}

// Layout invariants (Phase 1 cache observatory). An `Entity` is exactly one
// packed `u64`; eight of them fit in a cache line, which is the load on
// every dense storage iteration. Build fails immediately if anything below
// accidentally widens the struct.
const _: () = assert!(core::mem::size_of::<Entity>() == 8);
const _: () = assert!(core::mem::align_of::<Entity>() == 8);

/// Allocates and recycles entity indices, tracking a generation per slot.
#[derive(Debug, Default)]
pub(crate) struct EntityAllocator {
    generations: Vec<u32>,
    alive: Vec<bool>,
    free: Vec<u32>,
}

impl EntityAllocator {
    /// Allocates a fresh entity, recycling a dead index when one is available.
    pub fn alloc(&mut self) -> Entity {
        if let Some(index) = self.free.pop() {
            self.alive[index as usize] = true;
            Entity::from_parts(index, self.generations[index as usize])
        } else {
            let index = self.generations.len() as u32;
            self.generations.push(0);
            self.alive.push(true);
            Entity::from_parts(index, 0)
        }
    }

    /// Frees an entity, bumping its generation. Returns `false` if the handle
    /// was already stale or dead.
    pub fn free(&mut self, entity: Entity) -> bool {
        if !self.is_alive(entity) {
            return false;
        }
        let index = entity.index() as usize;
        self.alive[index] = false;
        self.generations[index] = self.generations[index].wrapping_add(1);
        self.free.push(entity.index());
        true
    }

    /// Returns `true` if `entity` refers to a currently live entity.
    pub fn is_alive(&self, entity: Entity) -> bool {
        let index = entity.index() as usize;
        index < self.alive.len()
            && self.alive[index]
            && self.generations[index] == entity.generation()
    }

    /// Reconstructs the live entity occupying `index`, if any.
    pub fn entity_at(&self, index: u32) -> Option<Entity> {
        let i = index as usize;
        if i < self.alive.len() && self.alive[i] {
            Some(Entity::from_parts(index, self.generations[i]))
        } else {
            None
        }
    }

    /// The number of currently live entities.
    pub fn live_count(&self) -> usize {
        self.alive.iter().filter(|&&a| a).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alloc_free_recycles_index_and_bumps_generation() {
        let mut alloc = EntityAllocator::default();
        let a = alloc.alloc();
        assert_eq!(a.index(), 0);
        assert_eq!(a.generation(), 0);
        assert!(alloc.is_alive(a));

        assert!(alloc.free(a));
        assert!(!alloc.is_alive(a)); // stale handle rejected

        let b = alloc.alloc();
        assert_eq!(b.index(), 0); // index recycled
        assert_eq!(b.generation(), 1); // generation bumped
        assert!(alloc.is_alive(b));
        assert!(!alloc.is_alive(a)); // old handle still rejected
    }

    #[test]
    fn double_free_is_rejected() {
        let mut alloc = EntityAllocator::default();
        let e = alloc.alloc();
        assert!(alloc.free(e));
        assert!(!alloc.free(e));
    }
}
