//! Bindless texture + sampler heap (ADR-044).
//!
//! Two persistent descriptor heaps:
//! - **Texture SRV heap** — capacity defaults to 16 384 (Tier 1 / RX 580
//!   baseline). 24-bit slot index + 8-bit generation packed into a u32
//!   [`BindlessTextureId`].
//! - **Sampler heap** — fixed capacity 64 with interning by
//!   [`crate::SamplerDesc`].
//!
//! All slot accounting (free-list, generation tags, overflow telemetry) is
//! engine-side; wgpu provides the underlying [`wgpu::BindGroup`] only.
//! Per ADR-044 §3 the free-list is LIFO (recently-freed slot reused first
//! so its descriptor stays hot in the GPU cache); fallback to the
//! monotonic `next_alloc` only when the free-list is empty.
//!
//! The module ships *without* GPU integration in PR 2 — the heap is a
//! data-structure exposing capacity / allocate / free / generation
//! semantics, plus the magenta-fallback slot. PR 3+ wires the heap to a
//! real `wgpu::BindGroup` once the deferred geometry pass needs it. This
//! ordering keeps PR 2's tests pure-Rust and headless-safe.

use crate::sampler::SamplerDesc;
use engine_core::collections::{DeterministicHasher, HashMap};

/// Index of the reserved "missing texture" fallback slot. Always allocated
/// at construction (a 1×1 magenta RGBA8 — the actual GPU upload lives in
/// PR 3 alongside the renderer that displays it).
pub const FALLBACK_TEXTURE_SLOT: u32 = 0;

/// Generation tag the fallback slot is stamped with. Picked so a freshly
/// constructed heap whose slot 0 has been overwritten by a user-uploaded
/// texture cannot be confused with the fallback by mistake.
pub const MAGENTA_FALLBACK_GENERATION: u8 = 0;

/// Packed bindless texture handle.
///
/// `[ 8 bits generation | 24 bits slot ]`. Slot 0 is reserved
/// ([`FALLBACK_TEXTURE_SLOT`]); valid user slots run `1..=capacity-1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct BindlessTextureId(u32);

impl BindlessTextureId {
    const SLOT_BITS: u32 = 24;
    const SLOT_MASK: u32 = (1 << Self::SLOT_BITS) - 1;

    /// Construct from a raw u32 (shader-side / hot-reload path). Caller
    /// must have produced the u32 from a prior `.as_u32()` on the same
    /// heap.
    pub const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }

    /// Construct from `(slot, generation)`. Panics in debug if `slot`
    /// exceeds 24 bits.
    pub const fn new(slot: u32, generation: u8) -> Self {
        debug_assert!(slot <= Self::SLOT_MASK);
        Self(((generation as u32) << Self::SLOT_BITS) | (slot & Self::SLOT_MASK))
    }

    /// Slot index (low 24 bits).
    pub const fn slot(self) -> u32 {
        self.0 & Self::SLOT_MASK
    }

    /// Generation tag (high 8 bits).
    pub const fn generation(self) -> u8 {
        (self.0 >> Self::SLOT_BITS) as u8
    }

    /// Raw 32-bit representation (the shader-side push-constant payload).
    pub const fn as_u32(self) -> u32 {
        self.0
    }
}

/// Packed bindless sampler handle.
///
/// The sampler heap has fixed capacity 64, so only 6 bits of slot are
/// used; the remainder of the u32 is reserved for a future generation /
/// reuse pattern.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct BindlessSamplerId(u32);

impl BindlessSamplerId {
    /// Construct from a raw u32.
    pub const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }

    /// Construct from a slot index.
    pub const fn new(slot: u32) -> Self {
        Self(slot)
    }

    /// Slot index.
    pub const fn slot(self) -> u32 {
        self.0
    }

    /// Raw 32-bit representation.
    pub const fn as_u32(self) -> u32 {
        self.0
    }
}

/// Configuration for [`BindlessHeap::new`].
#[derive(Clone, Copy, Debug)]
pub struct BindlessHeapConfig {
    /// Capacity of the texture SRV heap. Default per ADR-044 §1: 16 384
    /// for Tier 1 / RX 580. Higher tiers scale up via
    /// [`crate::DeviceLimits::bindless_texture_capacity`].
    pub texture_capacity: u32,
    /// Capacity of the sampler heap. ADR-044 §1 fixes this at 64; the
    /// field is exposed so unit tests can shrink it to exercise overflow
    /// without allocating 64 distinct samplers per run.
    pub sampler_capacity: u32,
    /// Soft-cap fill ratio numerator. ADR-044 §4: 80%. Stored as a
    /// numerator over 100 so integer arithmetic suffices.
    pub soft_cap_percent: u32,
}

impl Default for BindlessHeapConfig {
    fn default() -> Self {
        Self {
            texture_capacity: 16_384,
            sampler_capacity: 64,
            soft_cap_percent: 80,
        }
    }
}

/// Snapshot of heap state for telemetry / oracle assertions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BindlessHeapStats {
    /// Live texture-slot count (including the reserved fallback slot).
    pub textures_used: u32,
    /// Configured texture capacity.
    pub textures_capacity: u32,
    /// Live unique sampler count.
    pub samplers_used: u32,
    /// Configured sampler capacity.
    pub samplers_capacity: u32,
    /// Number of hard-cap overflow events observed since construction.
    pub overflow_events: u32,
    /// Number of soft-cap events observed since construction.
    pub soft_cap_events: u32,
}

/// Error returned when a heap insertion can't be satisfied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HeapFull;

impl core::fmt::Display for HeapFull {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("bindless heap full (ADR-044 §4 hard cap)")
    }
}

impl std::error::Error for HeapFull {}

/// Persistent bindless heap.
///
/// Texture slots are u32-indexed; sampler slots interned by descriptor. Both
/// are dense; allocation prefers the LIFO free-list.
///
/// PR 2 ships the pure data-structure surface — `insert_texture`,
/// `free_texture`, `intern_sampler`, `stats`. PR 3 binds the heap to the
/// actual `wgpu::BindGroup` once the renderer needs descriptor-array
/// reads.
pub struct BindlessHeap {
    config: BindlessHeapConfig,
    // Per-slot generation tag. Indexed by slot. Slot 0 starts at
    // `MAGENTA_FALLBACK_GENERATION` and never increments (the fallback is
    // immortal). Other slots increment on every free → reuse cycle.
    generations: Vec<u8>,
    // `true` if the slot is currently allocated.
    occupied: Vec<bool>,
    // LIFO free-list of slots returned by `free_texture`.
    free_list: Vec<u32>,
    // Monotonic next-fresh-slot pointer; consulted when `free_list` is empty.
    next_alloc: u32,
    // Slot count actually in use (includes the reserved fallback).
    textures_used: u32,
    // Sampler interning: SamplerDesc → slot index. Uses the engine's
    // owned deterministic hasher so the assignment order is identical
    // across runs (ADR-013 / engine_core::collections).
    sampler_slots: HashMap<SamplerDesc, u32, DeterministicHasher>,
    samplers_used: u32,
    // Telemetry counters. EVENT emission lands in PR 3 when the renderer
    // wires `engine_core::telemetry`; PR 2 surfaces the counts via
    // [`BindlessHeapStats`].
    overflow_events: u32,
    soft_cap_events: u32,
    // Whether the next non-fallback insertion has already triggered the
    // soft-cap event (sticky once fired; cleared by `clear_soft_cap`).
    soft_cap_latched: bool,
}

impl BindlessHeap {
    /// Construct a heap with default capacities (16 384 textures, 64
    /// samplers).
    pub fn new() -> Self {
        Self::with_config(BindlessHeapConfig::default())
    }

    /// Construct a heap with explicit configuration.
    pub fn with_config(config: BindlessHeapConfig) -> Self {
        assert!(
            config.texture_capacity >= 1,
            "BindlessHeap texture_capacity must be ≥ 1 (slot 0 reserved)"
        );
        assert!(
            config.texture_capacity <= (1 << BindlessTextureId::SLOT_BITS),
            "BindlessHeap texture_capacity exceeds 24-bit slot range"
        );
        assert!(
            config.sampler_capacity >= 1,
            "BindlessHeap sampler_capacity must be ≥ 1"
        );
        let cap = config.texture_capacity as usize;
        let mut generations = vec![0u8; cap];
        let mut occupied = vec![false; cap];
        // Reserve the fallback slot.
        generations[FALLBACK_TEXTURE_SLOT as usize] = MAGENTA_FALLBACK_GENERATION;
        occupied[FALLBACK_TEXTURE_SLOT as usize] = true;
        Self {
            config,
            generations,
            occupied,
            free_list: Vec::new(),
            next_alloc: 1, // skip the reserved fallback at slot 0
            textures_used: 1,
            sampler_slots: HashMap::with_hasher(DeterministicHasher::new()),
            samplers_used: 0,
            overflow_events: 0,
            soft_cap_events: 0,
            soft_cap_latched: false,
        }
    }

    /// Reserve a fresh texture slot.
    ///
    /// On success returns a [`BindlessTextureId`] whose generation tag
    /// matches the slot's current generation. The caller is expected to
    /// upload the wgpu::Texture to the slot via the descriptor array
    /// (PR 3+).
    ///
    /// Soft cap (ADR-044 §4): when the live count crosses
    /// `config.soft_cap_percent` of capacity, `soft_cap_events` is bumped
    /// (latched until [`Self::clear_soft_cap`]). PR 3's telemetry callback
    /// reads the latch and emits the EVENT.
    ///
    /// Hard cap: when no slot is available, returns `Err(HeapFull)`. The
    /// `overflow_events` counter is bumped; the asset server substitutes
    /// the fallback magenta at [`FALLBACK_TEXTURE_SLOT`].
    pub fn insert_texture(&mut self) -> Result<BindlessTextureId, HeapFull> {
        let slot = if let Some(s) = self.free_list.pop() {
            s
        } else if self.next_alloc < self.config.texture_capacity {
            let s = self.next_alloc;
            self.next_alloc += 1;
            s
        } else {
            self.overflow_events = self.overflow_events.saturating_add(1);
            return Err(HeapFull);
        };
        self.occupied[slot as usize] = true;
        let generation = self.generations[slot as usize];
        self.textures_used = self.textures_used.saturating_add(1);
        // Soft-cap check: percent-of-capacity threshold (integer math).
        let threshold = self
            .config
            .texture_capacity
            .saturating_mul(self.config.soft_cap_percent)
            / 100;
        if !self.soft_cap_latched && self.textures_used >= threshold && threshold > 0 {
            self.soft_cap_latched = true;
            self.soft_cap_events = self.soft_cap_events.saturating_add(1);
        }
        Ok(BindlessTextureId::new(slot, generation))
    }

    /// Free a previously-inserted texture slot.
    ///
    /// On free the slot's generation tag is incremented (with wrapping at
    /// 256 — ADR-044 "Risks and tradeoffs" notes 256 reuses per slot
    /// before wrap, decades at any realistic asset churn). Subsequent
    /// reads of the old [`BindlessTextureId`] mismatch in the high 8 bits
    /// and the renderer can `debug_assert` on stale references.
    ///
    /// Freeing [`FALLBACK_TEXTURE_SLOT`] is a no-op (the fallback is
    /// permanent).
    pub fn free_texture(&mut self, id: BindlessTextureId) {
        let slot = id.slot();
        if slot == FALLBACK_TEXTURE_SLOT {
            return;
        }
        if (slot as usize) >= self.occupied.len() {
            return;
        }
        if !self.occupied[slot as usize] {
            return;
        }
        // Generation mismatch — caller is freeing a stale handle. Drop on
        // the floor (no error surface; the asset server tolerates this).
        if self.generations[slot as usize] != id.generation() {
            return;
        }
        self.occupied[slot as usize] = false;
        self.generations[slot as usize] = self.generations[slot as usize].wrapping_add(1);
        self.free_list.push(slot);
        self.textures_used = self.textures_used.saturating_sub(1);
    }

    /// Look up the current generation tag for a slot. Used by PR 3+
    /// shader-side debug asserts; engine code rarely calls it.
    pub fn slot_generation(&self, slot: u32) -> u8 {
        self.generations
            .get(slot as usize)
            .copied()
            .unwrap_or_default()
    }

    /// Is the slot currently allocated.
    pub fn is_slot_occupied(&self, slot: u32) -> bool {
        self.occupied.get(slot as usize).copied().unwrap_or(false)
    }

    /// Intern a sampler descriptor, returning the existing slot if the
    /// descriptor has already been seen.
    ///
    /// On hard-cap overflow (more than `config.sampler_capacity` distinct
    /// descriptors) returns `Err(HeapFull)` and bumps `overflow_events`;
    /// the asset server substitutes the default linear-repeat sampler at
    /// slot 0 (ADR-044 §5).
    pub fn intern_sampler(&mut self, desc: SamplerDesc) -> Result<BindlessSamplerId, HeapFull> {
        if let Some(slot) = self.sampler_slots.get(&desc) {
            return Ok(BindlessSamplerId::new(*slot));
        }
        if self.samplers_used >= self.config.sampler_capacity {
            self.overflow_events = self.overflow_events.saturating_add(1);
            return Err(HeapFull);
        }
        let slot = self.samplers_used;
        self.sampler_slots.insert(desc, slot);
        self.samplers_used += 1;
        Ok(BindlessSamplerId::new(slot))
    }

    /// Snapshot of heap state for telemetry / oracle assertions.
    pub fn stats(&self) -> BindlessHeapStats {
        BindlessHeapStats {
            textures_used: self.textures_used,
            textures_capacity: self.config.texture_capacity,
            samplers_used: self.samplers_used,
            samplers_capacity: self.config.sampler_capacity,
            overflow_events: self.overflow_events,
            soft_cap_events: self.soft_cap_events,
        }
    }

    /// Clear the soft-cap latch so the next crossing emits another event.
    /// Called by PR 3's telemetry wiring after the EVENT has been flushed.
    pub fn clear_soft_cap(&mut self) {
        self.soft_cap_latched = false;
    }
}

impl core::fmt::Debug for BindlessHeap {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BindlessHeap")
            .field("stats", &self.stats())
            .finish_non_exhaustive()
    }
}

impl Default for BindlessHeap {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sampler::{AddressMode, FilterMode, SamplerDesc};

    #[test]
    fn id_packing_round_trips() {
        let id = BindlessTextureId::new(0xABCDEF, 0x42);
        assert_eq!(id.slot(), 0xABCDEF);
        assert_eq!(id.generation(), 0x42);
        assert_eq!(id.as_u32(), (0x42 << 24) | 0xABCDEF);
        assert_eq!(BindlessTextureId::from_raw(id.as_u32()), id);
    }

    #[test]
    fn fallback_slot_is_reserved_at_construction() {
        let heap = BindlessHeap::new();
        let stats = heap.stats();
        assert_eq!(stats.textures_used, 1, "fallback slot is live");
        assert!(heap.is_slot_occupied(FALLBACK_TEXTURE_SLOT));
        assert_eq!(
            heap.slot_generation(FALLBACK_TEXTURE_SLOT),
            MAGENTA_FALLBACK_GENERATION
        );
    }

    #[test]
    fn allocate_walks_next_alloc_until_free_list_fills() {
        let mut heap = BindlessHeap::with_config(BindlessHeapConfig {
            texture_capacity: 8,
            sampler_capacity: 4,
            soft_cap_percent: 80,
        });
        let a = heap.insert_texture().unwrap();
        let b = heap.insert_texture().unwrap();
        let c = heap.insert_texture().unwrap();
        // Monotonic-fresh allocation after the reserved slot 0.
        assert_eq!([a.slot(), b.slot(), c.slot()], [1, 2, 3]);
    }

    #[test]
    fn free_increments_generation_and_lifo_reuses_slot() {
        let mut heap = BindlessHeap::with_config(BindlessHeapConfig {
            texture_capacity: 4,
            sampler_capacity: 4,
            soft_cap_percent: 80,
        });
        let a = heap.insert_texture().unwrap();
        assert_eq!(a.generation(), 0);
        heap.free_texture(a);
        let a2 = heap.insert_texture().unwrap();
        assert_eq!(
            a2.slot(),
            a.slot(),
            "LIFO reuses the most-recently-freed slot"
        );
        assert_eq!(a2.generation(), 1, "generation tag incremented on free");
    }

    #[test]
    fn stale_free_after_generation_bump_is_dropped() {
        let mut heap = BindlessHeap::with_config(BindlessHeapConfig {
            texture_capacity: 4,
            sampler_capacity: 4,
            soft_cap_percent: 80,
        });
        let a = heap.insert_texture().unwrap();
        heap.free_texture(a);
        let _a2 = heap.insert_texture().unwrap();
        // Old `a` handle now stale (generation mismatch). Drop it on the floor.
        heap.free_texture(a);
        // The freshly-allocated slot is still live.
        assert!(heap.is_slot_occupied(a.slot()));
    }

    #[test]
    fn soft_cap_event_fires_once_per_crossing() {
        // capacity = 10, soft cap = 80% → threshold 8 textures (incl. fallback).
        let mut heap = BindlessHeap::with_config(BindlessHeapConfig {
            texture_capacity: 10,
            sampler_capacity: 4,
            soft_cap_percent: 80,
        });
        for _ in 0..6 {
            heap.insert_texture().unwrap();
        }
        // textures_used now 7 (1 fallback + 6 user); below threshold of 8.
        assert_eq!(heap.stats().soft_cap_events, 0);
        heap.insert_texture().unwrap(); // crosses to 8
        assert_eq!(heap.stats().soft_cap_events, 1);
        heap.insert_texture().unwrap(); // already latched; no second event
        assert_eq!(heap.stats().soft_cap_events, 1);
        heap.clear_soft_cap();
        heap.insert_texture().unwrap(); // re-arms latch
        assert_eq!(heap.stats().soft_cap_events, 2);
    }

    #[test]
    fn hard_cap_returns_heap_full_and_bumps_counter() {
        let mut heap = BindlessHeap::with_config(BindlessHeapConfig {
            texture_capacity: 3,
            sampler_capacity: 4,
            soft_cap_percent: 100, // disable soft cap interference
        });
        let _ = heap.insert_texture().unwrap(); // slot 1
        let _ = heap.insert_texture().unwrap(); // slot 2
        let err = heap.insert_texture().unwrap_err();
        assert_eq!(err, HeapFull);
        assert_eq!(heap.stats().overflow_events, 1);
    }

    #[test]
    fn freeing_the_fallback_slot_is_a_noop() {
        let mut heap = BindlessHeap::with_config(BindlessHeapConfig {
            texture_capacity: 4,
            sampler_capacity: 4,
            soft_cap_percent: 80,
        });
        heap.free_texture(BindlessTextureId::new(
            FALLBACK_TEXTURE_SLOT,
            MAGENTA_FALLBACK_GENERATION,
        ));
        assert!(heap.is_slot_occupied(FALLBACK_TEXTURE_SLOT));
        assert_eq!(heap.stats().textures_used, 1);
    }

    #[test]
    fn sampler_intern_dedupes_by_descriptor() {
        let mut heap = BindlessHeap::with_config(BindlessHeapConfig {
            texture_capacity: 4,
            sampler_capacity: 4,
            soft_cap_percent: 80,
        });
        let a = heap.intern_sampler(SamplerDesc::linear_repeat()).unwrap();
        let b = heap.intern_sampler(SamplerDesc::linear_repeat()).unwrap();
        assert_eq!(a, b, "structurally-equal descriptors share a slot");
        let c = heap.intern_sampler(SamplerDesc::nearest_clamp()).unwrap();
        assert_ne!(a, c);
        assert_eq!(heap.stats().samplers_used, 2);
    }

    #[test]
    fn sampler_intern_hard_cap() {
        let mut heap = BindlessHeap::with_config(BindlessHeapConfig {
            texture_capacity: 4,
            sampler_capacity: 2,
            soft_cap_percent: 80,
        });
        let _ = heap.intern_sampler(SamplerDesc::linear_repeat()).unwrap();
        let _ = heap.intern_sampler(SamplerDesc::nearest_clamp()).unwrap();
        // Third unique descriptor would overflow.
        let third = SamplerDesc {
            address_u: AddressMode::MirrorRepeat,
            address_v: AddressMode::MirrorRepeat,
            address_w: AddressMode::MirrorRepeat,
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            mipmap_filter: FilterMode::Linear,
            anisotropy: 4,
            comparison: false,
        };
        assert_eq!(heap.intern_sampler(third), Err(HeapFull));
        assert_eq!(heap.stats().overflow_events, 1);
    }
}
