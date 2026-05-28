# ADR-064 — GPU `record()` contracts: geometry + lighting passes

- Status: Accepted (Phase 6 design contract; implementation lands in
  Phase 6 PR 3)
- Date: 2026-05-27
- Phase: 6 — RENDERING FOUNDATION (Track A, Part 2)
- Companion: ADR-039 (render graph), ADR-040 (CSM cascade + atlas),
  ADR-043 (cluster lights binning), ADR-044 (bindless heap),
  ADR-046 (oracle regression criteria), ADR-049 (engine-gpu wrapper),
  ADR-063 (shader artefact binding), ADR-068 (Phase 6 PR slicing)

## Context

Phase 5 PR 3 (commit `aad7a84`) shipped five Track-A passes —
`CullPass`, `CsmShadowPass`, `GBufferPass`, `ClusterLightPass`,
`LightingAccumulationPass` — as `engine_render::passes` entries with
no-op `record()` bodies. Their CPU reference implementations live in
`testbed/engine-raster/src/{cull,shadow,cluster,shading}.rs`. The
oracle (ADR-046) verifies the CPU reference against the source-of-
truth math (cluster grid extents, CSM cascade splits, Cook-Torrance
BRDF).

Phase 6 PR 3 turns the no-op `record()` bodies into real wgpu calls.
This is the "first visible image" PR — at PR 3's end, an
`engine-bench-frame-pacing` invocation on the original CI runner
should rasterize a real scene through the deferred pipeline.

The work is contained: ADR-039 gives the graph; ADR-049 gives the
wgpu wrapper; ADR-044 gives the bindless heap; ADRs 040 + 043 give
the CSM and cluster algorithms. ADR-063 gives the shader binding.
What this ADR fixes is the *GPU contract surface* — descriptor
layouts, push-constant payload, vertex buffer layout, MRT format
choice, SSBO layout — so the five passes interoperate at the byte
level with the shaders + the asset format.

## Decision

### 1. Vertex buffer layout (consumed by `CullPass`, `GBufferPass`,
`CsmShadowPass`)

The vertex shader's input layout matches the EMSH `semantic_mask`
(ADR-061 §1) bit-order:

```text
@location(0) position:    vec3<f32>;   // bit 0
@location(1) normal:      vec3<f32>;   // bit 1
@location(2) tangent:     vec4<f32>;   // bit 2 (w = sign)
@location(3) uv0:         vec2<f32>;   // bit 3
@location(4) uv1:         vec2<f32>;   // bit 4
@location(5) color0:      vec4<f32>;   // bit 5
@location(6) bone_weight: vec4<f32>;   // bit 6
@location(7) bone_index:  vec4<u32>;   // bit 7
```

A shader compiled with `--vertex-mask 0b00001111` (position + normal +
tangent + uv0) consumes only the present locations; the engine's
pipeline construction (ADR-063) emits a `VertexBufferLayout` whose
stride exactly matches the EMSH's `vertex_stride`.

### 2. Push-constant payload (consumed by every geometry pass)

64-byte push-constants per ADR-063 §5:

```text
PushConstants (64 B):
  model_xform     [f32; 12]    // 3x4 affine (48 B)
  material_index  u32           // index into bindless EMAT pool (4 B)
  instance_id     u32           // for indirect draws (4 B)
  flags           u32           // per-draw bitset (4 B)
  reserved        u32           // pad (4 B)
```

### 3. MRT G-buffer format (`GBufferPass`)

Per ADR-039's `GBufferAlbedoRoughness` / `GBufferNormalMetallic` /
`GBufferMotionDepth` + ADR-044 considerations:

| Slot | Resource type            | Format                  | Notes                |
|------|--------------------------|-------------------------|----------------------|
| 0    | GBufferAlbedoRoughness   | `Bgra8UnormSrgb`        | sRGB-aware           |
| 1    | GBufferNormalMetallic    | `Rgba16Float`           | normal in .xyz, metallic .w |
| 2    | GBufferMotionDepth       | `Rgba16Float`           | motion .xy, view-depth .z, ID .w |
| -    | DepthBuffer              | `Depth32Float`          | reverse-Z (ADR-040 §3) |

`Bgra8UnormSrgb` matches the swapchain's expected color format and
avoids conversion in tonemap. `Rgba16Float` for normal+metallic is
the smallest format that preserves normal precision; `Rg16Snorm`
would save 4 bytes per pixel but adds quantization error visible at
glancing angles.

The `Rgba16Float` motion+depth target packs the per-pixel ID into
`.w` so the `LightingAccumulationPass` can look up the material via
bindless without a separate ID buffer.

### 4. CSM atlas layout (`CsmShadowPass`)

Per ADR-040 §3 exactly: 4096² `Depth32Float`, reverse-Z, four
quadrants:

```
+----+----+
| C0 | C1 |    each 2048×2048
+----+----+
| C2 | C3 |
+----+----+
```

The atlas is created once at device init and re-cleared every frame.
The cascade view-projection matrices are uploaded to a uniform buffer
in Group 1 (`per-pass`) at pass entry:

```text
CsmUniforms (Group 1, 256 B):
  cascade_vp        [mat4x4<f32>; 4]   // 256 B
  cascade_splits    [f32; 4]            // (z-range upper bounds)
  filter_radius_px  f32
  bias_constant     f32
  bias_slope        f32
  cascade_count     u32
```

The four cascades render into the atlas via four single-cascade
sub-passes, each binding the corresponding viewport. Reverse-Z
depth-only — no color attachment.

### 5. Cluster light SSBO layout (`ClusterLightPass`)

Per ADR-043 §1 + ADR-043 §3, the 16×9×24 grid is flattened into a
linear SSBO of 3456 cells. Each cell stores its assigned light count
and an offset into a parallel `light_indices` buffer.

```text
ClusterCells SSBO (~6 KiB):
  cells[3456]:
    light_offset  u32         // index into light_indices
    light_count   u32

LightIndices SSBO (~441 KiB max):
  indices[3456 * 32]: u32     // up to 32 lights per cluster (cap)

LightData SSBO (per ADR-043 §2; up to 256 lights total):
  lights[256]:
    position_radius   vec4<f32>     // .xyz position, .w radius
    color_intensity   vec4<f32>     // .rgb color, .a intensity
    direction         vec4<f32>     // .xyz direction (spot/sun)
    params            vec4<f32>     // .x inner-cone, .y outer-cone, .z falloff, .w type
```

`ClusterLightPass` is a compute pass: 24 workgroups of (16, 9) =
3456 threads, each cell processes the assigned light list against the
cluster's view-space frustum. The CPU oracle (`cluster.rs`) is the
parity reference.

### 6. Lighting accumulation (`LightingAccumulationPass`)

Reads G-buffers (groups 1+2 via bindless), reads ClusterCells +
LightData + ShadowAtlas, evaluates per-pixel Cook-Torrance GGX +
Smith-Schlick (matching `testbed/engine-raster/src/shading.rs`'s
math), accumulates into `LitColor` (`Rgba16Float`).

The pass is a full-screen triangle (no vertex buffer; vertex shader
generates positions from `vertex_index`). Push-constants carry the
inverse view-projection matrix + the screen extent.

### 7. CullPass (compute)

Workgroup size: (64, 1, 1). Reads `RenderQueue` (per-instance world
AABBs); writes `IndirectDrawBuffer` (per-instance `DrawIndexedIndirect`
command + an atomically incremented draw count). The frustum is
extracted from the view-projection matrix and tested in compute. The
CPU oracle equivalent (`testbed/engine-raster/src/scene.rs`'s
`Frustum::contains_aabb`) is the parity reference.

## Rationale

- **The CPU oracle is the math source.** PR 3 does not invent any
  rendering math — it transposes the CPU oracle's algorithms onto
  GPU compute / draw paths and verifies pixel parity. Every decision
  in this ADR aligns to a corresponding `testbed/engine-raster`
  module.
- **MRT format choices are the smallest layouts preserving the
  oracle's precision.** Tested by rendering an oracle fixture and
  checking the channel-precision deltas; `Rgba16Float` for normals
  is the established AAA-renderer choice and matches the oracle's
  internal `[f32; 3]` precision after the float→half quantization.
- **64 B push-constants + bindless materials = stateless draws.** The
  draw doesn't need a per-material BindGroup binding step; the shader
  reads `material_index` from push-constants and indexes the bindless
  EMAT pool. This is the descriptor-set-free rendering path ADR-044
  was designed for.
- **CSM as four sub-passes is simpler than instanced cascade rendering.**
  Modern GPUs handle multi-pass rendering of small viewports with
  minimal overhead (the cascade renders are the cheapest passes in
  the frame). Instanced rendering would save a few draw-call submits
  at the cost of shader complexity; not worth it for 4 cascades.
- **ClusterLightPass as compute + SSBO** is the standard Forward+
  variant adapted to deferred. The 32-lights-per-cluster cap is
  ADR-043's; the SSBO sizing is determined by the cap.

## Consequences

- The five passes' Phase-5 stubs gain `pipeline:
  engine_gpu::RenderPipeline` (or `ComputePipeline`) fields, plus
  bind-group / vertex-buffer / target descriptors per their
  ADR-064 contracts.
- Three new oracle fixtures land in `testbed/engine-raster/tests/`:
  `cube_pixel_parity`, `csm_4_cascade_pixel_parity`,
  `cluster_64_lights_pixel_parity`. Each renders the same fixture
  via CPU oracle + GPU path and asserts pixel parity at ADR-046's
  thresholds (1/255 channel, p99 ≤ 1%).
- The pipeline cache (ADR-063) sees 5 pipeline-construction misses on
  first frame: ~50 ms total on the original CI runner. PR 6's
  frame-pacing metrics report frame 0 separately.
- A new `crates/engine-render/shaders/` directory hosts the five
  passes' Slang sources (`cull.slang`, `csm_shadow.slang`,
  `gbuffer.slang`, `cluster_assign.slang`, `lighting.slang`). The
  pre-build script invokes `tools/engine-shader/` to emit `.bundle`
  artefacts; the runtime loads them through the asset pipeline.
- The oracle exception register (`docs/audit/oracle-exceptions.md`)
  may gain entries for documented vendor-driver divergences. None
  expected on AMD AMDGPU (the runner's GPU), but driver updates
  during the implementation may introduce noise.

## Risks and tradeoffs

- **The reverse-Z depth comparison must be consistent.** Every pass
  that samples or writes the depth buffer must agree on the
  `LessEqual` / `GreaterEqual` flip (reverse-Z uses
  `CompareFunction::Greater`). One inconsistent pass produces
  invisible geometry. Mitigation: a shared `pipeline_defaults.rs`
  module emits the right `DepthStencilState` for every geometry
  pass; the convention is one place.
- **`Rgba16Float` motion+depth is 8 B/pixel — at 4K, the G-buffer is
  192 MiB before tile compression.** RDNA2-class GPU has 12 GiB VRAM,
  fine; lower-tier hardware (the RX 580 milestone) at 8 GiB starts
  to feel it. Mitigation: ADR-045's bindless heap reports
  texture-VRAM usage; if PR 6's bench shows >50% VRAM on the RX 580,
  PR 6.5 may pack motion+depth into `Rg16Float` + `R32Float`.
- **ClusterLightPass is the most complex compute pass.** Mistakes
  in the per-cell light-index allocation can produce light leaking
  between cells. Mitigation: the CPU oracle's `cluster_assignment_
  oracle` test catches this exact class of bug — pixel parity
  enforces correctness.
- **CSM filtering is Vogel-disk 16-tap PCF (ADR-040 §5).** A naive
  GPU port can produce non-deterministic ordering of the 16 samples;
  the WGSL `loop` with a static unroll hint keeps it deterministic.
  The oracle fixture `shadow_heavy_scene` catches drift.

## Alternatives considered

- **Forward+ (per-pixel light loop without G-buffer).** Lower
  bandwidth on simple scenes; loses the deferred property that
  most Phase-6 fixtures rely on. Rejected — spec §IV.4.A pins
  deferred as Track A.
- **Visibility-buffer rendering instead of G-buffer.** Higher
  performance ceiling; requires mesh-shader path (Track B). Out of
  scope for Phase 6 per the plan's "strict Track A close" decision.
- **Single big shader for the entire deferred chain.** Loses the
  per-pass `record()` modularity. Rejected — ADR-039's graph is
  the chosen abstraction.
- **128 B push-constants (full Vulkan budget).** Loses WebGPU
  portability. Rejected per ADR-063 §5.
- **Async-compute overlap of CullPass with CsmShadowPass.** Spec
  §IV.4.A line 385 names async-compute as a Phase 6+ optimization;
  in Phase 6 PR 3 the queue is single (graphics+compute on one
  `wgpu::Queue`). Async lands as a Phase 7 optimization ADR.

## Verification

- Implementation lands in Phase 6 PR 3. Test files:
  - `tests/rendering/cube_pixel_parity.rs`: render a textured cube via
    CPU oracle and GPU path; assert ADR-046 pixel parity.
  - `tests/rendering/csm_4_cascade_pixel_parity.rs`: shadow-heavy
    scene; CPU vs GPU parity on the 4096² atlas's contents (sampled
    and compared per cascade quadrant).
  - `tests/rendering/cluster_64_lights_pixel_parity.rs`: 64-light
    scene; CPU vs GPU parity on the LitColor target.
  - `tests/rendering/gbuffer_format_roundtrip.rs`: write known data
    into each G-buffer; read back via a debug pass; assert
    quantization is within documented bounds.
- The CI `determinism` job runs the new tests on x86-64 and aarch64
  (the CPU oracle path is the cross-arch reference; the GPU path
  runs only on the self-hosted GPU runner and is documented as
  hardware-specific).
- Manual runbook: after PR 3, render `combined_deferred_scene` via
  `engine-bench-frame-pacing --scene testbed/frame-pacing/scenes/v0.ron`
  and capture frame 0 to `docs/observatory/phase-6-pr-3-frame-zero.png`
  as a milestone artifact.
- The wgpu boundary guard (ADR-049) is the boundary check; no new
  CI guard needed.
- Telemetry (ADR-010): per-pass `SPAN` markers
  (`SPAN "render.cull"`, `SPAN "render.csm"`, etc.) are emitted by
  the pass `record()` bodies; the frame-pacing JSON report ingests
  them for per-pass breakdown.
