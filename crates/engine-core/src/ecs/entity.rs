//! Entity identifiers and the entity allocator.

use super::archetype::ArchetypeId;
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

/// Where an entity currently lives in the archetype index (ADR-031).
///
/// A live entity is always associated with exactly one archetype — either
/// the empty archetype (no Table components attached) or a non-empty one.
/// Dead entities also carry a slot here but the value is meaningless until
/// the slot is re-allocated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct EntityLocation {
    /// The archetype the entity currently occupies.
    pub(crate) archetype: ArchetypeId,
    /// Row within that archetype's columns and `entity_indices` vector.
    pub(crate) row: u32,
}

impl EntityLocation {
    pub(crate) const fn empty() -> Self {
        Self {
            archetype: ArchetypeId::EMPTY,
            row: 0,
        }
    }
}

/// Allocates and recycles entity indices, tracking a generation per slot.
#[derive(Debug, Default)]
pub(crate) struct EntityAllocator {
    generations: Vec<u32>,
    alive: Vec<bool>,
    locations: Vec<EntityLocation>,
    free: Vec<u32>,
}

impl EntityAllocator {
    /// Allocates a fresh entity, recycling a dead index when one is available.
    pub fn alloc(&mut self) -> Entity {
        if let Some(index) = self.free.pop() {
            let i = index as usize;
            self.alive[i] = true;
            // Recycled slot: reset to the empty-archetype location so the
            // caller can safely look up `location_of(entity)` immediately.
            self.locations[i] = EntityLocation::empty();
            Entity::from_parts(index, self.generations[i])
        } else {
            let index = self.generations.len() as u32;
            self.generations.push(0);
            self.alive.push(true);
            self.locations.push(EntityLocation::empty());
            Entity::from_parts(index, 0)
        }
    }

    /// Frees an entity, bumping its generation. Returns `false` if the
    /// handle was already stale or dead.
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

    /// The archetype + row of `entity`, if alive.
    pub fn location_of(&self, entity: Entity) -> Option<EntityLocation> {
        if !self.is_alive(entity) {
            return None;
        }
        Some(self.locations[entity.index() as usize])
    }

    /// Updates the recorded location of `entity`. Caller is responsible for
    /// keeping it consistent with the archetype index (ADR-031).
    pub fn set_location(&mut self, entity: Entity, loc: EntityLocation) {
        debug_assert!(self.is_alive(entity));
        self.locations[entity.index() as usize] = loc;
    }

    /// Direct location read by raw index. Used when patching up a
    /// swap-removed entity's row in the same archetype: the entity's
    /// `Entity` handle is known by index only at that point.
    pub fn set_location_by_index(&mut self, index: u32, loc: EntityLocation) {
        debug_assert!(self.alive[index as usize]);
        self.locations[index as usize] = loc;
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
        assert_eq!(alloc.location_of(a), Some(EntityLocation::empty()));

        assert!(alloc.free(a));
        assert!(!alloc.is_alive(a)); // stale handle rejected

        let b = alloc.alloc();
        assert_eq!(b.index(), 0); // index recycled
        assert_eq!(b.generation(), 1); // generation bumped
        assert!(alloc.is_alive(b));
        assert!(!alloc.is_alive(a)); // old handle still rejected
        // Recycled slot is reset to the empty-archetype location.
        assert_eq!(alloc.location_of(b), Some(EntityLocation::empty()));
    }

    #[test]
    fn double_free_is_rejected() {
        let mut alloc = EntityAllocator::default();
        let e = alloc.alloc();
        assert!(alloc.free(e));
        assert!(!alloc.free(e));
    }
}
