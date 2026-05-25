# ADR-044 — Bindless texture heap allocation

- Status: Accepted (Phase 5 design contract; implementation lands in
  Phase 5 PR 2)
- Date: 2026-05-24
- Phase: 5 — RENDERING FOUNDATION (Track A)
- Companion: ADR-039 (render graph), ADR-045 (texture compression
  fallback), ADR-049 (engine-gpu boundary)

## Context

Spec §IV.4.A lines 402–404:

> All textures live in a single descriptor heap
> (`maxDescriptorSetSampledImages`, typically 16 K to 1 M on modern
> hardware). Texture ID is a `u32` index. Shaders sample as
> `textures[push.texture_id]`. Eliminates all per-material bind calls.

The spec fixes that *one* heap holds *all* textures and IDs are 32-
bit indices into it. Open questions:

- How is the heap allocated and how is fragmentation managed?
- What is the upper bound (16 K on the RX 580 milestone target;
  1 M+ on enthusiast/AAA tiers)?
- How are indices stable across frames (so an asset hot-reload
  doesn't invalidate every reference)?
- How are sampler states handled (separate heap? interleaved?)?
- Overflow behaviour when the heap fills.

Bindless is the foundation of every modern renderer (Frostbite,
Naughty Dog, Unreal 5). The choice space is shallow; this ADR
records the engine's variant.

## Decision

### 1. One descriptor heap, two views: SRV (textures) and sampler

The `engine-gpu` wrapper (ADR-049) exposes two persistent descriptor
heaps:
- `texture_srv_heap` — `[BindlessSrv; N_textures]`, default capacity
  16 384 (matching the RX 580 baseline; expandable to 65 536 / 262 144
  / 1 048 576 per hardware tier per spec Part XX.7).
- `sampler_heap` — `[BindlessSampler; 64]`, fixed capacity (samplers
  are coarse-grained; 64 covers every reasonable engine config).

Two heaps because Vulkan / D3D12 separate them; wgpu lets us bind
them as two `BindGroup`s on slot 0 (textures) and slot 1 (samplers).

### 2. Stable u32 indices, generation tags

```rust
#[repr(C)]
pub struct BindlessTextureId(u32);
```

The index is 24-bit (16 M textures, more than any real engine).
The high 8 bits are a generation tag — when a slot is freed and
reused, the generation increments; mismatches at read time are a
shader-side `debug_assert` (compiled out in release). Same pattern
as the ECS `Entity` (Phase 3, ADR-031).

Stability: textures keep their `BindlessTextureId` for their entire
lifetime in the engine. Hot-reload (ADR-008 content-addressed
pipeline) replaces the underlying `wgpu::Texture` while preserving
the slot. Asset references (`Handle<Texture>`) hold the
`BindlessTextureId`, not a pointer.

### 3. Slot allocation — free-list with first-fit

```
free_list: Vec<u32>            // freed slot indices (LIFO)
next_alloc: u32                // monotonic; consulted when free_list empty
```

A texture's slot is the first popped from the free-list, falling back
to `next_alloc++`. LIFO reuse improves descriptor cache locality
(recently-freed slot is hot in the GPU's descriptor cache).

### 4. Overflow behaviour — soft cap, hard cap, telemetry

- **Soft cap:** at 80% capacity (e.g. 13 107 of 16 384), emit a
  `EVENT "render.bindless_soft_cap"` telemetry signal and log a
  warning. Artists / level designers see this in the editor's
  PROFILING tab.
- **Hard cap:** at 100% capacity, `BindlessHeap::insert` returns
  `Err(HeapFull)`. The asset server (`engine_asset::AssetServer`)
  catches this and substitutes a fallback "missing texture" (1×1
  magenta, slot 0 — reserved). The original asset stays unloaded
  until a slot frees. A `COUNTER "render.bindless_overflow_total"`
  signal records each event.

Per-tier upgrades come from the bindless capacity being a
construction-time argument: `BindlessHeap::new(capacity: u32)`.
Project config (`engine.toml`) selects per the hardware compatibility
tier (spec Part XX.7).

### 5. Sampler state interning

Samplers are interned (`HashMap<SamplerDesc, SamplerId>`,
DeterministicHasher). 64 unique samplers should cover every engine
config (per-asset clamp/wrap × per-asset filter × per-asset
anisotropy → ~12 typical, with headroom).

A `COUNTER "render.sampler_intern_unique"` exposes the live count
so a runaway sampler explosion is visible.

### 6. Shader-side ABI

```hlsl
[[vk::binding(0, 0)]] Texture2D<float4> textures[];
[[vk::binding(1, 0)]] SamplerState samplers[];

float4 sample(uint texture_id, uint sampler_id, float2 uv) {
    return textures[texture_id].Sample(samplers[sampler_id], uv);
}
```

The texture / sampler IDs are pushed via push constants (8 bytes:
`(texture_id: u32, sampler_id: u32)`) per draw call — the only
per-draw state outside the indirect draw buffer (ADR-039's render
graph).

## Consequences

- Per-material data shrinks to `(albedo_id, normal_id, roughmet_id,
  ao_id, sampler_id) = 20 bytes`. Material asset format is correspondingly
  compact; an entire scene's materials fit in a single SSBO.
- The render graph (ADR-039) treats the bindless heap as a single
  long-lived `Resource<BindlessHeap>` — every pass that samples a
  texture reads it. No per-pass bind groups for material textures.
- `engine-gpu` (ADR-049) is the *only* crate that names the
  `wgpu::BindGroup` types; downstream crates see `BindlessTextureId`
  and call `gpu.allocate_texture(...)`.
- `BindlessHeap` is an owned-discipline component: the free-list,
  generation tags, and overflow telemetry are not delegated to wgpu.
  wgpu provides the descriptor heap; the slot accounting is engine-
  side.

## Risks and tradeoffs

- **24-bit slot + 8-bit generation in a u32.** 256 reuses per slot
  before generation wraps. With a 60 FPS / 16 K-texture engine,
  generation wrap is decades away. Acceptable.
- **First-fit free-list is not the cache-optimal allocation
  pattern.** A SLAB-style power-of-two bucketed allocator would be
  better for descriptor cache coherence but adds code. Phase 6+
  candidate.
- **Sampler heap is fixed at 64.** Unlikely to fill in practice; if
  it does, `SamplerHeap::insert` returns `Err(HeapFull)` and the
  default sampler (slot 0) is used. Telemetry COUNTER tracks.
- **Overflow → magenta fallback** is a visible art bug. Better than
  a crash, worse than a graceful unload-LRU. LRU eviction is a
  Phase 6+ candidate; Phase 5 ships the simpler soft/hard cap.
- **Bindless requires `descriptor_indexing` Vulkan feature** (mandatory
  in Vulkan 1.3, optional in 1.2 with `VK_EXT_descriptor_indexing`).
  Every GPU in the spec Part XX.7 compatibility tiers supports it;
  the RX 580 with Mesa RADV does. Documented as a hardware requirement.

## Alternatives considered

- **Traditional descriptor sets (one per material).** Per-draw bind
  cost is significant on the RX 580; bindless eliminates it. The
  spec mandates bindless.
- **Push-constant texture indices.** What this ADR does. Considered
  alternative: shader-resource arrays consumed via `NonUniformResource
  Index` (HLSL) — semantically the same; spelling differs.
- **Variable-rate slot allocation (descriptor heap regions per
  asset type — albedo, normal, etc.).** Improves cache coherence at
  the cost of separate per-type bookkeeping. Phase 6+ optimisation
  if profiler shows descriptor-cache misses.

## Verification

- Implementation lands with Phase 5 PR 2. Tests:
  - `tests/bindless_alloc.rs`: insert / free / reuse cycle; assert
    generation tags increment correctly; deterministic across
    runs.
  - `tests/bindless_overflow.rs`: exceed soft cap → telemetry event
    fires; exceed hard cap → `Err(HeapFull)` returned; fallback
    slot 0 is used.
  - `tests/bindless_handle_stability.rs`: insert texture, hot-
    reload underlying `wgpu::Texture`, assert `BindlessTextureId`
    unchanged.
- CI: the `wgpu::` boundary grep guard (ADR-049) keeps any direct
  `wgpu::BindGroup` use outside `engine-gpu/`.
- Telemetry: `GAUGE "render.bindless_textures_used"`,
  `GAUGE "render.bindless_textures_capacity"`,
  `COUNTER "render.bindless_overflow_total"`,
  `COUNTER "render.sampler_intern_unique"`.
