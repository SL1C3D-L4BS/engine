# engine-gpu

The owned wgpu wrapper (spec IV.1 Level 1; ADR-049).

## Purpose

`engine-gpu` is the **only** crate in the workspace permitted to name
`wgpu::*` types. Every higher-level renderer (`engine-render` today;
upper-layer Phase 6+ code later) consumes the engine's owned surface
through `engine_render::gpu` (= `pub use engine_gpu as gpu;`). The
boundary is enforced by a CI grep guard at
`.github/workflows/ci.yml` (ADR-049 §6); a `wgpu::` reference outside
this crate fails the build.

The owned wrapper exists to keep the renderer's API independent of
wgpu's evolution cadence: when wgpu lands a breaking change, the
patch lives in this one crate, and the engine's surface stays stable.
The same pattern applies in reverse to vendor SDK bindings — when
DLSS Streamline / FSR / XeSS SDKs ship, their wrappers will name
`engine_gpu::*` types, not `wgpu::*`.

## Modules

| Module       | Contents |
|--------------|----------|
| `device`     | `Device` — wraps `wgpu::Device + wgpu::Queue + wgpu::Instance`. Constructed via `Device::new()` (probes adapters in priority order) or `Device::new_with_surface(handle)` for swapchain hosts. |
| `queue`      | `Queue::submit`, `Queue::write_buffer`, `Queue::write_texture_2d` — the owned submit/upload surface. |
| `swapchain`  | `Swapchain` — surface + present configuration. Takes a `raw_window_handle::SurfaceTarget` at the boundary; everything beyond is owned. |
| `buffer`     | `Buffer` + `BufferUsage` (VERTEX / INDEX / UNIFORM / STORAGE / INDIRECT / COPY_SRC / COPY_DST). |
| `texture`    | `Texture` + `TextureFormat` (RGBA8 / RGBA8Srgb / RGBA16F / RGBA32F / R8 / R16F / R32F / DEPTH32F / BC{4,5,6,7}). |
| `sampler`    | `Sampler` + `SamplerKind` enum, interned via `DeterministicHasher` (engine-core). |
| `pipeline`   | `RenderPipeline`, `ComputePipeline`. Owned descriptors; no `wgpu::*` leaks. |
| `command`    | `CommandEncoder`, `RenderPass`, `ComputePass`. The encoder is the only handle that crosses the GPU boundary at runtime. |
| `bindless`   | `BindlessHeap` per ADR-044 — 24-bit slot + 8-bit generation, LIFO free-list, sampler interning, soft / hard cap telemetry, magenta-fallback slot 0. |
| `error`      | `GpuError` — owned error surface; no anyhow / no thiserror. |

## Design notes

- **Construction is fallible.** `Device::new()` walks the wgpu
  adapter list in `[primary > fallback]` order and returns the
  first that accepts the engine's feature mask. Web target (ADR-006)
  takes a different path through WebGPU and is Phase-6 work; today
  Linux + Vulkan via Mesa RADV is the reference.
- **No public access to `wgpu::Device`.** The internal `wgpu::Device`
  is held in `Arc<DeviceInner>` and exposed only through the engine's
  owned surface. The single inspection point is `Device::raw()` —
  *crate-private* in `engine-gpu`, used inside `bindless::` and
  `command::` modules. This keeps the boundary one type-line wide.
- **Bindless texture heap (ADR-044).** `BindlessHeap` issues stable
  `BindlessId` handles for textures and samplers. Slot 0 is reserved
  for the magenta-fallback texture so a missing-or-broken upload
  renders as a visually-obvious magenta block. The generation byte
  on each `BindlessId` defends against use-after-free. The free-list
  is LIFO so slot reuse is cache-friendly.
- **Texture compression import (ADR-045).** `TextureFormat::Bc{4,5,7}`
  carry the runtime side of the BC-import path. The
  `engine-asset::texture::TextureMeta` 24-byte header is duplicated
  rather than introducing a Level-1↔Level-1 dep on
  `engine_asset`. `tools/engine-tex-compress/` lands BC bytes at
  authoring time; the runtime importer reads the header and uploads
  through `Queue::write_texture_2d`.
- **Frame pacing visibility.** The wrapper does not own frame-time
  collection — `engine_platform::Instant::now()` is the canonical
  clock per ADR-016. The wrapper emits `SPAN("frame.total",
  Subsystem::Render)` per frame for the rolled-up p99/σ; ADR-047 §3
  is the consumer.
- **Test constraint.** The workspace `wgpu` dep is configured with
  `default-features = false, features = ["wgsl"]` to avoid pulling
  backend code into the CI image. Constructing a real `Device` in a
  unit test panics; engine-gpu's own tests carefully avoid it, and
  upstream consumers (e.g. `engine-render::tests::upscale_selection`)
  use a `select_with(predicate, logger)` helper to drive cascade
  logic without a Device.

## Out of scope

- **Vendor SDK bindings.** DLSS / FSR / XeSS Phase-6 work.
- **Vulkan ray-tracing.** Spec calls for hardware RT in Phase 7+;
  the wrapper has no `TraceRayPipeline` today.
- **Mesh-shader work-graph (Track B).** Phase 6 per spec IV.4.B.
- **Cross-platform Web target.** Phase 7+ per ADR-006; WebGPU surface
  construction is not yet wrapped.

## Oracle

`engine-gpu`'s own unit tests cover the type surface — descriptor
round-trips, format enumeration parity, sampler interning, bindless
slot generation, error display. The wrapper has no integration test
that constructs a `Device` (the workspace wgpu dep lacks backend
features). The substantive verification of the renderer's GPU path
is the frame-pacing bench (`bin/engine-bench-frame-pacing`) once the
self-hosted RX 6700 XT runner stands up.

## Dependencies

`wgpu` (workspace-pinned), `raw-window-handle` (swapchain surface
target). No `bytemuck`, no `pollster` — owned discipline.
