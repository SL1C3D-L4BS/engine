# ADR-077 — 3D Gaussian Splatting architecture (`engine-splatting`)

- Status: Accepted (Phase 6 design contract; implementation lands in
  Phase 6 PR 2)
- Date: 2026-05-28
- Phase: 6 — NEURAL RENDERING & GAUSSIAN SPLATTING
- Companion: ADR-039 (render-graph trait surface), ADR-046 (rasterizer
  oracle regression), ADR-049 (engine-gpu wrapper), ADR-053 (Phase 5
  precedent), ADR-064 (GPU geometry/lighting pass contracts),
  ADR-068 (prior Phase-6 slicing record — superseded by ADR-084),
  ADR-074 (wgpu Vulkan backend on Polaris), ADR-078 (ESPL format +
  glTF reader), ADR-084 (Phase 6 PR slicing — this ADR's parent)

## Context

ENGINE_SPECIFICATION_v2.0.md lines 1633–1637 name spec Phase 6's
deliverable:

> Portfolio: 3DGS renderer · Owned ONNX inference · Vendor upscaler
> integration. Milestone: 3DGS scene > 60 FPS.

3D Gaussian Splatting (Kerbl et al., SIGGRAPH 2023) is the
neural-rendering primitive the spec names as the Phase 6 portfolio
exhibit. Unlike raster-mesh rendering, a 3DGS scene is a point cloud
of *anisotropic Gaussians* — each splat carries a position, scale
(3-axis ellipsoid), rotation (quaternion), color, opacity, and a
spherical-harmonics representation of view-dependent appearance
(typically 9 SH coefficients per channel for the L=2 band).

Rendering a 3DGS scene is structurally different from forward or
deferred raster:

1. **No vertex/index buffers.** Each splat is one *primitive*: a 2D
   Gaussian footprint projected from the 3D anisotropic ellipsoid.
2. **No depth buffer.** Splats are *blended back-to-front* by depth
   relative to the camera (alpha over the running framebuffer);
   the depth-sort *is* the visibility test.
3. **Per-frame sort is the workhorse.** A 1M-splat scene re-sorts
   1M depth keys every frame. The sort dominates the frame budget
   if implemented naively; the canonical implementation uses
   parallel radix sort on the GPU.
4. **SH evaluation is per-fragment.** The view-dependent appearance
   uses 9 SH coefficients per RGB channel; the per-pixel evaluation
   is a fixed-function 27-multiply 27-add expansion.

The engine's Phase 5.5 closure shipped 11 Track-A passes for
deferred-PBR forward + post-FX (gbuffer, cluster_assign, csm, cull,
lighting, ibl, taa, ssao, bloom, tonemap, upscale). The 3DGS path is
*additive*: a new composite pass after the opaque chain that
alpha-blends sorted splats over the deferred framebuffer. The two
paths share the depth buffer (the splat composite reads opaque
depth to skip behind-opaque splats) and the post-FX chain (the
splat-composited HDR target is the input to TAA + tonemap).

The engine has no 3DGS code today. This ADR locks the architecture:
a new Level-2 crate `engine-splatting`, two new WGSL compute /
fragment shaders, a new CPU oracle module, a new ESPL asset format
(see ADR-078), three pixel-parity fixtures, and a CPU↔GPU
replay-parity oracle for the sort permutation.

## Decision

### 1. New Level-2 crate `engine-splatting`

```
crates/engine-splatting/
├── Cargo.toml
└── src/
    ├── lib.rs           # public surface
    ├── cloud.rs         # SplatCloud SoA storage
    ├── sort.rs          # parallel radix sort (CPU + GPU)
    ├── composite.rs     # render-graph Pass impl
    ├── asset.rs         # ESPL encode/decode (per ADR-078)
    ├── gltf_ext.rs      # KHR_gaussian_splatting reader
    └── contracts.rs     # push-constant + bind-group layouts
```

Workspace level 2: depends on `engine-math`, `engine-render`,
`engine-gpu`, `engine-asset`, `engine-platform`. No `engine-script`,
no `engine-editor`, no `engine-ui`.

### 2. `SplatCloud` storage — SoA per Acton DOD

The hot per-frame data is laid out as parallel arrays, cache-line
aligned, so the sort + composite both stream linearly:

```rust
pub struct SplatCloud {
    // Hot per-frame: read by sort.rs.
    position:   AlignedVec<Vec3>,    // [N] world-space center
    // Hot per-frame: read by composite.rs.
    scale:      AlignedVec<Vec3>,    // [N] ellipsoid axes (log-space)
    rotation:   AlignedVec<Quat>,    // [N] orientation quaternion
    color:      AlignedVec<Vec3>,    // [N] base RGB
    opacity:    AlignedVec<f32>,     // [N] alpha (logistic-coded on disk)
    // Optional view-dependent. 9 SH coefs per channel × 3 channels = 27 f32.
    sh:         Option<AlignedVec<[f32; 27]>>, // [N] or None for ambient-only clouds
    count: usize,
}
```

Each field is one allocation (`Box<[T]>` aligned to 64 bytes). No
`Vec<Splat>` AoS: the per-frame sort touches `position[]` only, and
the composite touches `scale[] | rotation[] | color[] | opacity[]`
+ optionally `sh[]`. AoS would pull 100+ bytes per splat into cache
when the sort needs 12 bytes; the SoA streams 12 contiguous bytes
per splat through L2.

`SplatCloud` exposes immutable accessors only; mutation goes through
`SplatCloudBuilder` (asset-loading path). No `clone()` — clouds are
loaded once via `Asset::decode()` and held by the renderer for the
scene's lifetime.

### 3. Parallel radix sort by depth

The per-frame work is dominated by `sort by camera-space depth` over
N splats (N ≈ 10⁶ for a benchmark scene). The chosen algorithm:
**4-pass × 8-bit radix sort** on the f32-bit-mangled depth key
(Pierce sign-flip + xor for negative range; CLRS Ch. 8.3 + parallel
section Ch. 27).

Two implementations:

```rust
// crates/engine-splatting/src/sort.rs

pub mod cpu {
    /// CPU reference: work-stealing over `engine_platform::ThreadPool`.
    /// Source of truth for the pixel-parity oracle.
    pub fn radix_sort_by_depth(
        cloud: &SplatCloud,
        view: Mat4,
    ) -> Vec<u32>; // permutation: indices sorted back-to-front
}

pub mod gpu {
    /// GPU radix sort via the splat_sort.wgsl compute shader.
    /// Single-pass 4-radix × 16-iter implementation, workgroup 256.
    pub fn radix_sort_by_depth(
        device: &engine_gpu::Device,
        encoder: &mut engine_gpu::Encoder,
        cloud_buffer: &Buffer,
        view: Mat4,
    ) -> Buffer; // permutation buffer, sorted back-to-front
}
```

**Determinism contract.** Both implementations must produce
byte-identical permutations across worker counts {1, 2, 4, N}. This
is the replay-parity discipline from ADR-033 applied to the sort
path; the pixel-parity fixtures depend on it.

Tie-breaking: when two splats have equal depth, the lower index
wins (stable sort). Standard radix-sort stability gives us this for
free on the CPU; the GPU implementation uses a per-pass stable
partition.

### 4. GPU composite pass

A new `SplatCompositePass` implements `engine_render::render_graph::Pass`:

- Inputs: the splat SoA buffers (read-only storage), the sorted
  permutation buffer (output of the sort pass), and the GBuffer
  depth (read-only sample to skip splats behind opaque geometry).
- Outputs: the HDR scene-color target, modified in place via
  back-to-front alpha-over blending.
- Pipeline kind: render pipeline (`@vertex` + `@fragment`), not
  compute. Each splat emits a single instance of a quad-billboard;
  the fragment shader evaluates the 2D Gaussian footprint and
  blends.

Track: A (graphics). Position in the graph: after `LightingPass`
and `IblPass`; before `TaaResolvePass`.

### 5. Spherical-harmonics evaluation

The L=2 SH (9 coefficients per channel) gives the per-splat
view-dependent appearance:

```
color(d) = c₀
         + c₁ Y₁⁻¹(d) + c₂ Y₁⁰(d) + c₃ Y₁¹(d)            // L=1, 3 dirs
         + c₄ Y₂⁻²(d) + ... + c₈ Y₂²(d)                   // L=2, 5 dirs
```

where `d` is the unit vector from the splat to the camera. The 9
basis functions are fixed-function polynomials in `d.{x,y,z}`. The
CPU oracle (`testbed/engine-raster/src/splat.rs`) and the GPU
fragment shader use the same evaluation form; the pixel-parity
fixture `splat_view_dependent` checks bit-by-bit on the SH math at
strict 1/255 over a sweep of 64 view directions.

### 6. Three pixel-parity fixtures

`crates/engine-splatting/tests/pixel_parity/`:

- **`splat_sphere.rs`** — single-splat scene with diffuse ambient.
  Verifies the 2D Gaussian footprint + the blend equation. Bound:
  strict 1/255.
- **`splat_garden_1m.rs`** — 1M-splat scene from the Kerbl et al.
  2023 reference release (the "garden" benchmark). Verifies sort
  correctness end-to-end. Bound: SSIM ≥ 0.95; the per-pixel parity
  threshold is relaxed because back-to-front alpha blending of 10⁶
  values has *inherent* f32-precision drift that exceeds 1/255 (the
  CPU oracle and GPU shader use the same math, but operation
  reorder in the GPU's hardware blend unit produces last-byte
  divergence per ADR-046 §3 vendor-driver category). This is the
  *first* SSIM-bound fixture in the engine; documented in the
  architectural-exception category from ADR-081's amendment to
  ADR-046.
- **`splat_view_dependent.rs`** — 16-splat scene with SH-coded
  appearance, camera orbit, 64 frames. Verifies SH evaluation
  (strict 1/255 on the SH math per-frame; SSIM ≥ 0.95 on the
  composite).

### 7. Sort replay-parity oracle

`crates/engine-splatting/tests/sort_replay_parity.rs`:

```rust
// For a fixed seed, the CPU sort produces the same permutation
// across worker counts. The GPU sort produces the same permutation
// as the CPU sort, byte-identical.
#[test]
fn cpu_sort_deterministic_across_workers() { ... }
#[test]
fn gpu_sort_matches_cpu() { ... }
```

This closes the replay-parity contract from ADR-033 over the new
sort surface.

### 8. WGSL shaders

Both new shaders live in `crates/engine-render/shaders/` (the
shader-artefact pipeline from ADR-063 owns shader storage; new
crates do not duplicate the path):

- **`splat_sort.wgsl`** — compute shader implementing the radix
  sort. Workgroup (256, 1, 1). Polaris-compatible: no subgroup
  intrinsics, no `f16`, only 32-bit `atomicAdd` on storage buffers
  (the digit-count phase of each radix iteration).
- **`splat_composite.wgsl`** — vertex + fragment shader. The vertex
  stage emits a billboard quad per instance; the fragment stage
  evaluates the 2D Gaussian and alpha-blends.

Both shaders flow through `engine_render::shader::wgsl_artefact_set`
and `build_render_pipeline` / `build_compute_pipeline` per ADR-063.

### 9. Importer subprocess

Per ADR-062's glTF-importer precedent, the 3DGS importer is a
*separate workspace member* under `tools/engine-splat-import/`
sandboxed via `engine_platform::sandbox`. It consumes `.ply` files
(the Kerbl reference release format) and `.splat` files (the INRIA
interchange format) and emits ESPL binaries + a JSON manifest. The
parse path is *outside* the engine binary; CI grep guard rejects
`ply::` / `splat_format::` usage outside this tool.

## Consequences

### Positive

- The 3DGS deliverable is realised: a measurable 1M-splat scene
  benchmark on the user's RX 580.
- The crate's SoA layout is a textbook DOD application; the
  per-frame sort + composite stream linearly through L2.
- The CPU oracle covers the SH math + the sort permutation at
  strict parity; the architectural-exception band only covers the
  inherent blend-order drift, not the algorithm itself.
- The renderer's render-graph trait surface absorbs the new pass
  without abstraction churn; no `Pass` API change.

### Negative

- A new Level-2 crate is one more workspace member with its own
  test surface. Mitigated by the strict crate-level decomposition
  (level 2, no upper-layer deps); the boundary stays clean.
- The SSIM-bound fixtures introduce a new fixture category to
  ADR-046. The "architectural divergence" amendment (ADR-081)
  formalises this, so the category is documented, not invented
  per-fixture.
- The 1M-splat fixture's reference data adds ~30 MiB of `.ply`
  source to the repo (tracked via Git LFS, same mechanism as the
  ONNX model from ADR-067). One more LFS pattern in
  `.gitattributes`.

### Neutral

- The KHR_gaussian_splatting glTF extension (Khronos, Aug 2025) is
  not yet ratified at this ADR's date; the reader implements the
  draft spec and rev-locks via a header comment. When ratification
  lands, the reader updates without external API churn.

## Implementation

PR 2 of Phase 6 (per ADR-084):

1. New crate `crates/engine-splatting/` with the public surface
   above.
2. New CPU oracle module `testbed/engine-raster/src/splat.rs`.
3. New shaders `crates/engine-render/shaders/splat_sort.wgsl` +
   `splat_composite.wgsl`.
4. New importer `tools/engine-splat-import/` with the
   ply/splat → ESPL pipeline.
5. Three pixel-parity fixtures + the sort replay-parity oracle.
6. `Cargo.toml` workspace member registration; `engine-render`'s
   integration tests pick up the new fixtures.

## References

### Books

- *Real-Time Rendering 4* (Akenine-Möller / Haines / Hoffman) —
  Ch. 8.5 (Alpha Blending) for the back-to-front compositing math;
  Ch. 5.2 (Polygonal Techniques) for the billboard quad emission.
- *Introduction to Algorithms* (Cormen / Leiserson / Rivest / Stein)
  — Ch. 8.3 (Radix Sort) + Ch. 27 (Parallel Algorithms) for the
  sort implementation.
- *Data-Oriented Design* (Acton, 2018) — the SoA storage layout
  pattern this crate applies.
- *Deep Learning* (Goodfellow / Bengio / Courville) — Ch. 14 on
  representation learning, the conceptual precursor to the SH-coded
  appearance per splat.

### Papers

- Kerbl, B., Kopanas, G., Leimkühler, T., Drettakis, G.
  *3D Gaussian Splatting for Real-Time Radiance Field Rendering*.
  ACM SIGGRAPH 2023, Vol. 42, No. 4.
  <https://repo-sam.inria.fr/fungraph/3d-gaussian-splatting/>.
- Original reference implementation:
  <https://github.com/graphdeco-inria/gaussian-splatting>.

### Standards

- Khronos KHR_gaussian_splatting glTF extension (Draft, August 2025).
  <https://github.com/KhronosGroup/glTF/tree/main/extensions/2.0/Khronos/KHR_gaussian_splatting>.

### Prior engine ADRs

- [ADR-039](039-render-graph-abstraction.md) — the `Pass` trait this
  ADR's composite pass implements.
- [ADR-046](046-rasterizer-oracle-regression.md) — the oracle bound
  this ADR's SSIM-band fixtures relax (per ADR-081's amendment).
- [ADR-049](049-engine-gpu-wgpu-wrapper.md) — the GPU surface the
  splat sort + composite consume.
- [ADR-064](064-gpu-pass-record-geometry-lighting.md) — the pass
  `record()` discipline.
- [ADR-074](074-wgpu-vulkan-backend-polaris.md) — the Polaris
  Vulkan backend the milestone bench runs on.
- [ADR-078](078-espl-format-and-gltf-extension.md) — the asset
  format this crate consumes.
- [ADR-081](081-oracle-exception-sunset-and-046-amendment.md) —
  the ADR-046 amendment that names the architectural-divergence
  category for blend-order drift.
- [ADR-084](084-phase-6-pr-slicing.md) — the Phase-6 PR slicing
  this ADR's implementation slot lives in.
