# engine-render

The two-track renderer's host crate (spec IV.1 Level 2, IV.4 Two-Track
Pipeline, IV.5 Frame Pacing Contract).

## Purpose

Owns the **render-graph trait surface** (ADR-039), the named pass set
for Track A (the deferred PBR path), and the upscaler trait surface
(ADR-005). The crate names only `engine_gpu::*` types on the GPU side
— `wgpu::*` is firewalled by the ADR-049 boundary CI guard. The
software rasterizer (Track A's CPU oracle, ADR-046) lives in
`testbed/engine-raster`, not here; the CPU reference is consumed via
test integration, not a runtime dep.

## Modules

| Module          | Contents |
|-----------------|----------|
| `render_graph`  | `Pass`, `PassContext`, `RenderGraph`, `Resource`, `ResourceId`, `ResourceKind`, `ResourceSet`, `Track` — the ADR-039 trait surface. |
| `passes`        | Track-A pass types: `CullPass`, `CsmShadowPass`, `ClusterLightPass`, `GBufferPass`, `LightingAccumulationPass`, `SsaoPass`, `IblPass`, `TaaPass`, `BloomPass`, `TonemapPass`, `UpscalePass`. Each names its read/write resources by tag. |
| `resources`     | Resource tag structs: G-buffer slices, shadow atlas, cluster cells, light SSBO, indirect-draw buffer, IBL probe set, BRDF LUT, SSAO texture, TAA history + resolved, bloom mips, tonemapped color, upscaled color. |
| `upscale`       | `UpscalerProvider` trait + four PR-5 providers (`VendorDlss` / `VendorFsr` / `VendorXess` stubs + `OwnedBilinear` placeholder) + `UpscalerRegistry` with vendor>best>owned selection per ADR-005. |
| (top of crate)  | `pub use engine_gpu as gpu;` — every `Device` / `Buffer` / `Texture` reference traverses this re-export so the ADR-049 boundary stays visible at the type level. |

## Design notes

- **Render-graph compile (ADR-039).** `RenderGraph::compile(track)`
  performs Kahn topological-sort on the registered passes' read/write
  resource tags. The order is total and deterministic; equal-rank
  nodes break ties on a stable pass name. Compilation emits an
  ordered `Vec<&dyn Pass>` ready to `record()`.
- **Track-A scheduling order (PR-3 + PR-4 + PR-5).** The canonical
  schedule is `cull → shadow/cluster → G-buffer → lighting → SSAO →
  IBL → TAA → upscale → bloom → tonemap`. Bloom reads pre-upscale TAA-
  resolved color and writes a post-upscale-resolution bloom buffer;
  the tonemap pass composites the upscaled HDR + the bloom into the
  final swapchain image. The 9-pass schedule test in
  `passes::tests::pr5_upscale_variant_schedules_taa_upscale_tonemap`
  pins this order.
- **GPU-pass `record()` is a Phase-6 deliverable.** PR 5 stops at the
  schedule + the trait surface. Each Track-A pass's `record()` body
  is a documented no-op; the GPU backend (real wgpu draw / dispatch
  calls) lands when the self-hosted runner stands up in Phase 6.
  The CPU oracle in `engine-raster` is the substantive reference
  until then.
- **Upscaler trait (ADR-005).** Four providers ship: DLSS, FSR, and
  XeSS as `supports() == false` vendor stubs (Phase-6 SDK bindings),
  plus `OwnedBilinear` as the universally-supported owned fallback.
  `UpscalerRegistry::select` walks providers in registration order
  and picks the first whose `supports(device)` accepts —
  vendor>best>owned per ADR-005 §Decision. Tests use
  `UpscalerRegistry::select_with(predicate, logger)` to drive the
  cascade without a real `Device` (which requires backend features
  the workspace CI does not enable).
- **Selection logging.** The `SelectionLogger` callback type
  (`&mut dyn FnMut(UpscalerKind)`) decouples the registry from
  `engine-telemetry`. The renderer wires it to the `ADR-010`
  telemetry channel; the bench binary captures it into the JSON
  report. The dependency-injection pattern keeps `engine-render` at
  Level 2 with no Level-1 telemetry dep.
- **Resource handles are tags, not values.** `Resource` is a thin
  marker; the actual GPU buffer / texture lives in
  `engine_gpu::BindlessHeap`. The graph routes ownership of an
  `engine_gpu::Buffer` / `engine_gpu::Texture` between passes via
  `BindlessId`; the trait surface here never owns a wgpu handle.

## Out of scope

- **Vendor SDK bindings** (DLSS Streamline 2.x, FSR 4 SDK, XeSS 2
  SDK). Phase 6 per ADR-005 §Consequences. The stubs ship now so the
  cascade selection is end-to-end-testable.
- **Owned ONNX temporal upscaler.** Phase 6+ per spec line 1634.
- **GPU-pass `record()` bodies.** The named passes compile but each
  body is a no-op. Phase 6 lands the GPU work when the runner is
  available.
- **Track B (mesh-shader work-graph).** Phase 6 per spec IV.4.B.
  The `Track` enum has the discriminant; no Track-B passes ship.
- **Render-thread topology.** The renderer is single-threaded for
  PR 5 / PR 6. The Phase-6 worker model lands with the GPU passes.

## Oracle

The crate's own integration tests:

- `tests/render_graph_topo.rs` — topological-sort determinism,
  read/write resource tracking, the canonical 9-pass schedule order
  for Track A (PR-1 + PR-3 + PR-4 + PR-5).
- `tests/upscale_selection.rs` — the ADR-005 cascade priority
  invariants: empty registry → None, all-false predicate → None,
  bilinear falls through when vendors decline, vendor wins when
  supported first, logger fires exactly once.

The CPU oracle for the bilinear placeholder lives in
`testbed/engine-raster/tests/upscale_oracle.rs` per ADR-005
§Verification (render-at-low-res, upscale, compare under L1 bound).

## Dependencies

`engine-gpu` only — re-exported as `engine_render::gpu` so the
ADR-049 boundary is visible at the call site. No direct `wgpu`
dependency; no `engine-telemetry` dependency (callback injection).
