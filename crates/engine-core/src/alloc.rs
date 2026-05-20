//! Arena allocators.
//!
//! The engine prefers arenas over scattered heap allocations: they make
//! lifetimes explicit, keep related data contiguous, and give the memory
//! debugger (spec XVI) clean per-arena accounting. Four backends are
//! provided here:
//!
//! - [`LinearArena`] — bump allocation of raw bytes, freed all at once by
//!   [`reset`](LinearArena::reset). The per-frame scratch allocator.
//! - [`RingArena`] — a fixed-capacity ring; pushing past capacity evicts the
//!   oldest entry. Backs rolling histories.
//! - [`PoolArena`] — a generational slot pool; stable handles, O(1) insert
//!   and remove.
//! - [`GeneralArena`] — a general-purpose free-list allocator with segregated
//!   size classes and a coalescing large-object list. Fills the gap when an
//!   allocation pattern is too irregular for a bump or pool.
//!
//! Every arena implements the [`Arena`] trait, which exposes a uniform
//! [`ArenaStats`] snapshot. Phase 2's `engine-memdbg` consumes these stats
//! through a global registry — the trait and the `#[track_caller]`
//! `with_capacity_named` constructors are the Phase 1 surface that lets that
//! work land additively.

use std::collections::VecDeque;

/// A snapshot of an [`Arena`]'s accounting at a moment in time.
///
/// Units are arena-specific:
///
/// - [`LinearArena`] reports bytes for `used`, `capacity`, and `peak`.
/// - [`RingArena`] and [`PoolArena`] report element counts.
/// - [`GeneralArena`] reports bytes.
///
/// The counters (`allocations`, `frees`, `resets`) are monotonic since
/// creation and never decrease.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ArenaStats {
    /// A static label for tooling. Defaults to the arena's type name when
    /// the caller did not supply one.
    pub name: &'static str,
    /// Currently allocated bytes or elements.
    pub used: usize,
    /// Maximum capacity in bytes or elements.
    pub capacity: usize,
    /// High-water mark of [`used`](Self::used).
    pub peak: usize,
    /// Total allocations since creation.
    pub allocations: u64,
    /// Total frees / evictions since creation.
    pub frees: u64,
    /// Total bulk resets since creation. Always `0` on arenas that don't
    /// support reset (`RingArena`, `PoolArena`).
    pub resets: u64,
}

/// Uniform accounting interface for every arena.
///
/// `engine-memdbg` (Phase 2, spec XVI) will register every live arena via
/// this trait and surface per-arena watermarks in the editor. Phase 1 only
/// exposes the interface; a global registry is intentionally deferred.
pub trait Arena {
    /// A snapshot of the arena's accounting.
    fn stats(&self) -> ArenaStats;

    /// The static name supplied at construction.
    fn name(&self) -> &'static str;
}

// --- LinearArena -----------------------------------------------------

/// A bump allocator over a fixed byte buffer.
#[derive(Debug)]
pub struct LinearArena {
    buffer: Vec<u8>,
    used: usize,
    peak: usize,
    allocations: u64,
    resets: u64,
    name: &'static str,
}

impl LinearArena {
    /// Creates an arena holding `capacity` bytes.
    #[track_caller]
    pub fn with_capacity(capacity: usize) -> Self {
        Self::with_capacity_named(capacity, "LinearArena")
    }

    /// Creates an arena holding `capacity` bytes with a static label for
    /// tooling. The `#[track_caller]` attribute lets `engine-memdbg`
    /// attribute the allocation to a source location in Phase 2.
    #[track_caller]
    pub fn with_capacity_named(capacity: usize, name: &'static str) -> Self {
        Self {
            buffer: vec![0u8; capacity],
            used: 0,
            peak: 0,
            allocations: 0,
            resets: 0,
            name,
        }
    }

    /// Allocates `len` bytes aligned to `align` (which must be a power of two).
    ///
    /// Returns `None` if the arena is exhausted.
    pub fn alloc(&mut self, len: usize, align: usize) -> Option<&mut [u8]> {
        assert!(align.is_power_of_two(), "alignment must be a power of two");
        let start = (self.used + align - 1) & !(align - 1);
        let end = start.checked_add(len)?;
        if end > self.buffer.len() {
            return None;
        }
        self.used = end;
        self.peak = self.peak.max(end);
        self.allocations += 1;
        Some(&mut self.buffer[start..end])
    }

    /// Frees every allocation at once. The peak watermark is retained.
    pub fn reset(&mut self) {
        self.used = 0;
        self.resets += 1;
    }

    /// Bytes currently allocated.
    pub fn used(&self) -> usize {
        self.used
    }

    /// Total capacity in bytes.
    pub fn capacity(&self) -> usize {
        self.buffer.len()
    }

    /// The high-water mark of [`used`](Self::used) since creation.
    pub fn peak(&self) -> usize {
        self.peak
    }

    /// A read-only view of the bytes currently in use. Lets tooling (e.g. the
    /// cache observatory and the Phase 2 memory debugger) inspect arena
    /// contents without touching internals.
    pub fn used_bytes(&self) -> &[u8] {
        &self.buffer[..self.used]
    }
}

impl Arena for LinearArena {
    fn stats(&self) -> ArenaStats {
        ArenaStats {
            name: self.name,
            used: self.used,
            capacity: self.buffer.len(),
            peak: self.peak,
            allocations: self.allocations,
            frees: 0,
            resets: self.resets,
        }
    }

    fn name(&self) -> &'static str {
        self.name
    }
}

// --- RingArena -------------------------------------------------------

/// A fixed-capacity ring; pushing past capacity evicts the oldest element.
#[derive(Debug)]
pub struct RingArena<T> {
    items: VecDeque<T>,
    capacity: usize,
    dropped: u64,
    pushes: u64,
    peak: usize,
    name: &'static str,
}

impl<T> RingArena<T> {
    /// Creates a ring holding at most `capacity` elements.
    #[track_caller]
    pub fn with_capacity(capacity: usize) -> Self {
        Self::with_capacity_named(capacity, "RingArena")
    }

    /// Creates a ring holding at most `capacity` elements with a static
    /// label for tooling.
    #[track_caller]
    pub fn with_capacity_named(capacity: usize, name: &'static str) -> Self {
        assert!(capacity > 0, "ring capacity must be non-zero");
        Self {
            items: VecDeque::with_capacity(capacity),
            capacity,
            dropped: 0,
            pushes: 0,
            peak: 0,
            name,
        }
    }

    /// Pushes `value`, returning the evicted oldest element if the ring was
    /// already full.
    pub fn push(&mut self, value: T) -> Option<T> {
        self.pushes += 1;
        let evicted = if self.items.len() == self.capacity {
            self.dropped += 1;
            self.items.pop_front()
        } else {
            None
        };
        self.items.push_back(value);
        self.peak = self.peak.max(self.items.len());
        evicted
    }

    /// Iterates the ring from oldest to newest.
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.items.iter()
    }

    /// Removes and returns every element, oldest first, leaving the ring
    /// empty. The overflow count from [`dropped`](Self::dropped) is retained.
    pub fn drain(&mut self) -> Vec<T> {
        let out: Vec<T> = self.items.drain(..).collect();
        self.dropped += out.len() as u64;
        out
    }

    /// The number of elements currently held.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Returns `true` if the ring holds no elements.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The ring's fixed capacity.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Total number of elements evicted by overflow since creation.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }
}

impl<T> Arena for RingArena<T> {
    fn stats(&self) -> ArenaStats {
        ArenaStats {
            name: self.name,
            used: self.items.len(),
            capacity: self.capacity,
            peak: self.peak,
            allocations: self.pushes,
            frees: self.dropped,
            resets: 0,
        }
    }

    fn name(&self) -> &'static str {
        self.name
    }
}

// --- PoolArena -------------------------------------------------------

/// A stable handle into a [`PoolArena`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PoolId {
    index: u32,
    generation: u32,
}

struct PoolSlot<T> {
    value: Option<T>,
    generation: u32,
}

/// A generational slot pool.
///
/// Handles stay valid until the slot is removed; a removed-then-reused slot
/// gets a fresh generation, so a stale [`PoolId`] is rejected rather than
/// silently aliasing a different value.
pub struct PoolArena<T> {
    slots: Vec<PoolSlot<T>>,
    free: Vec<u32>,
    inserts: u64,
    removes: u64,
    live: usize,
    peak: usize,
    name: &'static str,
}

impl<T> Default for PoolArena<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> PoolArena<T> {
    /// Creates an empty pool.
    #[track_caller]
    pub fn new() -> Self {
        Self::new_named("PoolArena")
    }

    /// Creates an empty pool with a static label for tooling.
    #[track_caller]
    pub fn new_named(name: &'static str) -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
            inserts: 0,
            removes: 0,
            live: 0,
            peak: 0,
            name,
        }
    }

    /// Inserts a value, returning a stable handle to it.
    pub fn insert(&mut self, value: T) -> PoolId {
        self.inserts += 1;
        self.live += 1;
        self.peak = self.peak.max(self.live);
        if let Some(index) = self.free.pop() {
            let slot = &mut self.slots[index as usize];
            slot.value = Some(value);
            PoolId {
                index,
                generation: slot.generation,
            }
        } else {
            let index = self.slots.len() as u32;
            self.slots.push(PoolSlot {
                value: Some(value),
                generation: 0,
            });
            PoolId {
                index,
                generation: 0,
            }
        }
    }

    /// Borrows the value behind `id`, or `None` if the handle is stale.
    pub fn get(&self, id: PoolId) -> Option<&T> {
        let slot = self.slots.get(id.index as usize)?;
        if slot.generation == id.generation {
            slot.value.as_ref()
        } else {
            None
        }
    }

    /// Mutably borrows the value behind `id`, or `None` if the handle is stale.
    pub fn get_mut(&mut self, id: PoolId) -> Option<&mut T> {
        let slot = self.slots.get_mut(id.index as usize)?;
        if slot.generation == id.generation {
            slot.value.as_mut()
        } else {
            None
        }
    }

    /// Removes and returns the value behind `id`, freeing its slot.
    pub fn remove(&mut self, id: PoolId) -> Option<T> {
        let slot = self.slots.get_mut(id.index as usize)?;
        if slot.generation != id.generation {
            return None;
        }
        let value = slot.value.take();
        if value.is_some() {
            slot.generation = slot.generation.wrapping_add(1);
            self.free.push(id.index);
            self.removes += 1;
            self.live -= 1;
        }
        value
    }

    /// The number of live values in the pool.
    pub fn len(&self) -> usize {
        self.live
    }

    /// Returns `true` if the pool holds no values.
    pub fn is_empty(&self) -> bool {
        self.live == 0
    }
}

impl<T> Arena for PoolArena<T> {
    fn stats(&self) -> ArenaStats {
        ArenaStats {
            name: self.name,
            used: self.live,
            capacity: self.slots.len(),
            peak: self.peak,
            allocations: self.inserts,
            frees: self.removes,
            resets: 0,
        }
    }

    fn name(&self) -> &'static str {
        self.name
    }
}

// --- GeneralArena ----------------------------------------------------

/// A general-purpose free-list allocator over a fixed byte buffer.
///
/// Small allocations (`<= 4096` bytes) route to one of eight segregated
/// size classes — 16, 32, 64, 128, 256, 512, 1024, 2048, 4096 — each with
/// its own free list. Large allocations route to a coalescing first-fit
/// list. Freed blocks return to their size class (or, for large blocks,
/// merge with neighbours). Returns `None` on exhaustion; the arena never
/// asks the heap for more memory.
///
/// This is the fourth arena promised by ADR-013's allocator layout (spec
/// XVI). It exists for allocation patterns too irregular for a bump arena
/// and not type-uniform enough for a pool — texture staging buffers, asset
/// hot-load scratch, the script-VM heap. See ADR-026.
pub struct GeneralArena {
    buffer: Vec<u8>,
    used: usize,
    peak: usize,
    allocations: u64,
    frees: u64,
    resets: u64,
    name: &'static str,
    free_lists: [u32; SIZE_CLASS_COUNT],
    large_head: u32,
    // The arena lives at a stable address while borrowed; we treat
    // `buffer` as a flat byte arena with `BlockHeader`s interleaved.
}

const SIZE_CLASSES: [usize; 9] = [16, 32, 64, 128, 256, 512, 1024, 2048, 4096];
const SIZE_CLASS_COUNT: usize = SIZE_CLASSES.len();
const LARGE_THRESHOLD: usize = 4096;
const NULL_OFFSET: u32 = u32::MAX;
const HEADER_SIZE: usize = std::mem::size_of::<BlockHeader>();
const HEADER_ALIGN: usize = std::mem::align_of::<BlockHeader>();

#[derive(Clone, Copy, Debug)]
#[repr(C)]
struct BlockHeader {
    /// Total block size in bytes including the header.
    size: u32,
    /// Size class index, or `u32::MAX` for large blocks.
    class: u32,
    /// Free-list next pointer (offset into `buffer`), or `NULL_OFFSET`.
    next: u32,
    /// Coalescing neighbour (offset into `buffer`), or `NULL_OFFSET`.
    /// Only used by large blocks; small blocks within a class are uniformly
    /// sized and never coalesce.
    prev_phys: u32,
}

impl GeneralArena {
    /// Creates an arena holding `capacity` bytes.
    #[track_caller]
    pub fn with_capacity(capacity: usize) -> Self {
        Self::with_capacity_named(capacity, "GeneralArena")
    }

    /// Creates an arena holding `capacity` bytes with a static label.
    #[track_caller]
    pub fn with_capacity_named(capacity: usize, name: &'static str) -> Self {
        // The buffer is aligned to `BlockHeader`'s alignment so headers can
        // be stored anywhere within without extra padding.
        let cap = capacity.max(HEADER_SIZE + LARGE_THRESHOLD);
        let mut me = Self {
            buffer: vec![0u8; cap],
            used: 0,
            peak: 0,
            allocations: 0,
            frees: 0,
            resets: 0,
            name,
            free_lists: [NULL_OFFSET; SIZE_CLASS_COUNT],
            large_head: NULL_OFFSET,
        };
        me.init_single_large_block();
        me
    }

    fn init_single_large_block(&mut self) {
        // Start with one large block spanning the whole buffer.
        let size = self.buffer.len() as u32;
        self.write_header(
            0,
            BlockHeader {
                size,
                class: u32::MAX,
                next: NULL_OFFSET,
                prev_phys: NULL_OFFSET,
            },
        );
        self.large_head = 0;
    }

    /// Allocates `len` bytes aligned to at least `BlockHeader`'s alignment
    /// (8 bytes on every supported target). Returns `None` if the arena
    /// cannot satisfy the request.
    pub fn alloc(&mut self, len: usize) -> Option<&mut [u8]> {
        if len == 0 {
            // Zero-sized allocations have no semantic meaning here; reject
            // them rather than carve out an empty block.
            return None;
        }
        let class = class_for(len);
        let block_off = if let Some(c) = class {
            self.alloc_from_class(c, len)?
        } else {
            self.alloc_from_large(len)?
        };
        self.allocations += 1;
        let total = self.read_header(block_off).size as usize;
        self.used = self.used.saturating_add(total);
        self.peak = self.peak.max(self.used);
        let payload = block_off as usize + HEADER_SIZE;
        let payload_len = total - HEADER_SIZE;
        Some(&mut self.buffer[payload..payload + payload_len.min(len_padded(len))])
    }

    /// Frees the block previously returned by [`alloc`](Self::alloc). The
    /// slice must be exactly the one that allocation returned — passing a
    /// sub-slice or an arbitrary buffer pointer is undefined behaviour.
    ///
    /// # Safety
    ///
    /// `ptr` must be the start of an allocation made by this arena. The
    /// caller must ensure no other reference to the allocation exists.
    pub unsafe fn free(&mut self, ptr: *const u8) {
        let base = self.buffer.as_ptr() as usize;
        let p = ptr as usize;
        debug_assert!(p >= base + HEADER_SIZE && p < base + self.buffer.len());
        let payload_off = (p - base) as u32;
        let block_off = payload_off - HEADER_SIZE as u32;
        let hdr = self.read_header(block_off);
        self.used = self.used.saturating_sub(hdr.size as usize);
        self.frees += 1;
        if hdr.class == u32::MAX {
            self.free_large(block_off);
        } else {
            self.free_small(block_off, hdr.class as usize);
        }
    }

    /// Frees every allocation at once and re-coalesces the buffer into one
    /// large block.
    pub fn reset(&mut self) {
        self.used = 0;
        self.resets += 1;
        self.free_lists = [NULL_OFFSET; SIZE_CLASS_COUNT];
        self.large_head = NULL_OFFSET;
        self.init_single_large_block();
    }

    /// Bytes currently in use (including per-block headers).
    pub fn used(&self) -> usize {
        self.used
    }

    /// Total capacity in bytes.
    pub fn capacity(&self) -> usize {
        self.buffer.len()
    }

    /// High-water mark of [`used`](Self::used) since creation.
    pub fn peak(&self) -> usize {
        self.peak
    }

    // --- internal: size-class allocator -----------------------------

    fn alloc_from_class(&mut self, class: usize, _len: usize) -> Option<u32> {
        // Pop from the class's free list if non-empty.
        let head = self.free_lists[class];
        if head != NULL_OFFSET {
            let hdr = self.read_header(head);
            self.free_lists[class] = hdr.next;
            return Some(head);
        }
        // Otherwise carve a new block from the large pool.
        let total = (SIZE_CLASSES[class] + HEADER_SIZE) as u32;
        let off = self.carve_large(total)?;
        self.write_header(
            off,
            BlockHeader {
                size: total,
                class: class as u32,
                next: NULL_OFFSET,
                prev_phys: NULL_OFFSET,
            },
        );
        Some(off)
    }

    fn free_small(&mut self, block_off: u32, class: usize) {
        // Push back onto the class free list. Headers are reused; we do not
        // attempt cross-class coalescing.
        let mut hdr = self.read_header(block_off);
        hdr.next = self.free_lists[class];
        self.write_header(block_off, hdr);
        self.free_lists[class] = block_off;
    }

    // --- internal: large allocator -----------------------------------

    fn alloc_from_large(&mut self, len: usize) -> Option<u32> {
        let want = (HEADER_SIZE + len_padded(len)) as u32;
        let off = self.carve_large(want)?;
        let mut hdr = self.read_header(off);
        hdr.class = u32::MAX;
        hdr.next = NULL_OFFSET;
        self.write_header(off, hdr);
        Some(off)
    }

    /// First-fit search of the large list, splitting an oversized block.
    /// Returns the offset of a block whose `size >= want`, removed from the
    /// list.
    fn carve_large(&mut self, want: u32) -> Option<u32> {
        let mut prev = NULL_OFFSET;
        let mut cur = self.large_head;
        while cur != NULL_OFFSET {
            let hdr = self.read_header(cur);
            if hdr.size >= want {
                // Detach `cur` from the list.
                let next = hdr.next;
                if prev == NULL_OFFSET {
                    self.large_head = next;
                } else {
                    let mut p = self.read_header(prev);
                    p.next = next;
                    self.write_header(prev, p);
                }
                // Split if the remainder is large enough to hold a header
                // plus a useful payload (one size class's worth).
                let remainder = hdr.size.saturating_sub(want);
                let min_split = (HEADER_SIZE + SIZE_CLASSES[0]) as u32;
                if remainder >= min_split {
                    let tail_off = cur + want;
                    self.write_header(
                        tail_off,
                        BlockHeader {
                            size: remainder,
                            class: u32::MAX,
                            next: self.large_head,
                            prev_phys: cur,
                        },
                    );
                    self.large_head = tail_off;
                    // Shrink `cur` to the requested size.
                    let mut h = self.read_header(cur);
                    h.size = want;
                    self.write_header(cur, h);
                }
                return Some(cur);
            }
            prev = cur;
            cur = hdr.next;
        }
        None
    }

    fn free_large(&mut self, block_off: u32) {
        // Push onto the large list (no coalescing yet; do a sweep instead).
        let mut hdr = self.read_header(block_off);
        hdr.class = u32::MAX;
        hdr.next = self.large_head;
        self.write_header(block_off, hdr);
        self.large_head = block_off;
        self.coalesce_large();
    }

    /// Sort-and-coalesce sweep of the large list. Quadratic in list length;
    /// for the workloads expected on this arena (a few thousand outstanding
    /// large blocks at most) that is acceptable, and it keeps the
    /// implementation small.
    fn coalesce_large(&mut self) {
        // Gather (offset, size) pairs.
        let mut blocks: Vec<(u32, u32)> = Vec::new();
        let mut cur = self.large_head;
        while cur != NULL_OFFSET {
            let hdr = self.read_header(cur);
            blocks.push((cur, hdr.size));
            cur = hdr.next;
        }
        blocks.sort_by_key(|&(off, _)| off);
        // Merge adjacent blocks.
        let mut i = 0;
        while i + 1 < blocks.len() {
            let (a_off, a_size) = blocks[i];
            let (b_off, b_size) = blocks[i + 1];
            if a_off + a_size == b_off {
                blocks[i] = (a_off, a_size + b_size);
                blocks.remove(i + 1);
            } else {
                i += 1;
            }
        }
        // Rewrite the free list with the coalesced blocks.
        self.large_head = NULL_OFFSET;
        for (off, size) in blocks.into_iter().rev() {
            self.write_header(
                off,
                BlockHeader {
                    size,
                    class: u32::MAX,
                    next: self.large_head,
                    prev_phys: NULL_OFFSET,
                },
            );
            self.large_head = off;
        }
    }

    // --- internal: raw header read/write ----------------------------

    fn read_header(&self, off: u32) -> BlockHeader {
        let off = off as usize;
        let ptr = self.buffer.as_ptr().wrapping_add(off) as *const BlockHeader;
        // SAFETY: `off` is always a multiple of `HEADER_ALIGN` (carving
        // rounds up to `len_padded`, which is a multiple of 8). The buffer
        // lives at least as long as `&self`.
        unsafe { ptr.read_unaligned() }
    }

    fn write_header(&mut self, off: u32, hdr: BlockHeader) {
        let off = off as usize;
        let ptr = self.buffer.as_mut_ptr().wrapping_add(off) as *mut BlockHeader;
        // SAFETY: as above; `&mut self` ensures exclusive access.
        unsafe { ptr.write_unaligned(hdr) };
    }
}

impl Arena for GeneralArena {
    fn stats(&self) -> ArenaStats {
        ArenaStats {
            name: self.name,
            used: self.used,
            capacity: self.buffer.len(),
            peak: self.peak,
            allocations: self.allocations,
            frees: self.frees,
            resets: self.resets,
        }
    }

    fn name(&self) -> &'static str {
        self.name
    }
}

fn class_for(len: usize) -> Option<usize> {
    if len > LARGE_THRESHOLD {
        return None;
    }
    SIZE_CLASSES.iter().position(|&c| c >= len)
}

/// Round `len` up to a `HEADER_ALIGN` multiple so the next header lands on
/// an aligned offset.
fn len_padded(len: usize) -> usize {
    let m = HEADER_ALIGN;
    (len + m - 1) & !(m - 1)
}

// --- tests -----------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_arena_bumps_and_resets() {
        let mut arena = LinearArena::with_capacity(256);
        let a = arena.alloc(64, 16).unwrap();
        assert_eq!(a.len(), 64);
        let _ = arena.alloc(64, 16).unwrap();
        assert_eq!(arena.used(), 128);
        assert!(arena.alloc(200, 1).is_none()); // exhausted
        arena.reset();
        assert_eq!(arena.used(), 0);
        assert_eq!(arena.peak(), 128); // watermark retained
    }

    #[test]
    fn linear_arena_respects_alignment() {
        let mut arena = LinearArena::with_capacity(256);
        let _ = arena.alloc(1, 1).unwrap();
        let aligned = arena.alloc(8, 64).unwrap();
        let addr = aligned.as_ptr() as usize;
        let base = arena.buffer.as_ptr() as usize;
        assert_eq!((addr - base) % 64, 0);
    }

    #[test]
    fn linear_arena_stats() {
        let mut arena = LinearArena::with_capacity_named(256, "test");
        let _ = arena.alloc(32, 4);
        let _ = arena.alloc(64, 4);
        arena.reset();
        let s = arena.stats();
        assert_eq!(s.name, "test");
        assert_eq!(s.capacity, 256);
        assert_eq!(s.used, 0);
        assert_eq!(s.peak, 96);
        assert_eq!(s.allocations, 2);
        assert_eq!(s.resets, 1);
    }

    #[test]
    fn ring_arena_evicts_oldest() {
        let mut ring = RingArena::with_capacity(3);
        assert_eq!(ring.push(1), None);
        assert_eq!(ring.push(2), None);
        assert_eq!(ring.push(3), None);
        assert_eq!(ring.push(4), Some(1)); // 1 evicted
        assert_eq!(ring.iter().copied().collect::<Vec<_>>(), vec![2, 3, 4]);
        assert_eq!(ring.dropped(), 1);
    }

    #[test]
    fn ring_arena_stats() {
        let mut ring = RingArena::with_capacity_named(2, "ring");
        let _ = ring.push(1);
        let _ = ring.push(2);
        let _ = ring.push(3);
        let s = ring.stats();
        assert_eq!(s.name, "ring");
        assert_eq!(s.capacity, 2);
        assert_eq!(s.used, 2);
        assert_eq!(s.peak, 2);
        assert_eq!(s.allocations, 3);
        assert_eq!(s.frees, 1);
    }

    #[test]
    fn pool_arena_rejects_stale_handles() {
        let mut pool: PoolArena<&str> = PoolArena::new();
        let id = pool.insert("alpha");
        assert_eq!(pool.get(id), Some(&"alpha"));
        assert_eq!(pool.remove(id), Some("alpha"));
        assert_eq!(pool.get(id), None); // stale

        let id2 = pool.insert("beta");
        assert_eq!(id2.index, id.index); // slot recycled
        assert_ne!(id2.generation, id.generation); // generation bumped
        assert_eq!(pool.get(id), None); // old handle still rejected
        assert_eq!(pool.get(id2), Some(&"beta"));
    }

    #[test]
    fn pool_arena_stats() {
        let mut pool: PoolArena<u32> = PoolArena::new_named("pool");
        let a = pool.insert(1);
        let _b = pool.insert(2);
        let _c = pool.insert(3);
        pool.remove(a);
        let s = pool.stats();
        assert_eq!(s.name, "pool");
        assert_eq!(s.used, 2);
        assert_eq!(s.peak, 3);
        assert_eq!(s.allocations, 3);
        assert_eq!(s.frees, 1);
    }

    #[test]
    fn general_arena_size_class_round_trip() {
        let mut g = GeneralArena::with_capacity(64 * 1024);
        let mut handles: Vec<*const u8> = Vec::new();
        for _ in 0..32 {
            let b = g.alloc(64).expect("alloc");
            handles.push(b.as_ptr());
        }
        assert_eq!(g.stats().allocations, 32);
        for &p in &handles {
            unsafe { g.free(p) };
        }
        assert_eq!(g.stats().frees, 32);
        // After freeing everything the used count returns to zero.
        assert_eq!(g.stats().used, 0);
        // A subsequent allocation of the same class re-uses a freed slot.
        let _ = g.alloc(64).expect("alloc");
        assert_eq!(g.stats().allocations, 33);
    }

    #[test]
    fn general_arena_handles_every_class() {
        let mut g = GeneralArena::with_capacity(512 * 1024);
        for &sz in &SIZE_CLASSES {
            let b = g.alloc(sz).expect("class fits");
            assert_eq!(b.len(), sz);
        }
    }

    #[test]
    fn general_arena_large_first_fit_and_coalesce() {
        let mut g = GeneralArena::with_capacity(64 * 1024);
        let a = g.alloc(8000).expect("a") as *mut [u8];
        let b = g.alloc(8000).expect("b") as *mut [u8];
        let c = g.alloc(8000).expect("c") as *mut [u8];
        // Free middle, then ends, then verify a fresh 16k+ alloc succeeds
        // (it can only fit because freed neighbours coalesced).
        unsafe {
            g.free((*b).as_ptr());
            g.free((*a).as_ptr());
            g.free((*c).as_ptr());
        }
        let big = g.alloc(20000).expect("after coalesce");
        assert_eq!(big.len(), 20000);
    }

    #[test]
    fn general_arena_exhaustion_returns_none() {
        let mut g = GeneralArena::with_capacity(8 * 1024);
        let _ = g.alloc(4096).expect("first");
        // The remainder is now too small for another 4096 + header; the
        // size-class allocator should fail rather than panic.
        assert!(g.alloc(4096).is_none());
    }

    #[test]
    fn general_arena_reset_zeroes_used() {
        let mut g = GeneralArena::with_capacity(16 * 1024);
        let _ = g.alloc(256);
        let _ = g.alloc(1024);
        let used_before = g.stats().used;
        assert!(used_before > 0);
        g.reset();
        let s = g.stats();
        assert_eq!(s.used, 0);
        assert_eq!(s.resets, 1);
    }
}
