# ADR-063 — Slang artefact to GPU pipeline binding

- Status: Accepted (Phase 6 design contract; implementation lands in
  Phase 6 PR 2)
- Date: 2026-05-27
- Phase: 6 — RENDERING FOUNDATION (Track A, Part 2)
- Companion: ADR-037 (Slang toolchain — artefact source), ADR-039
  (render graph), ADR-044 (bindless heap — descriptor layout cap),
  ADR-049 (engine-gpu wrapper — pipeline construction surface),
  ADR-061 (EMAT — material's `shader_id` resolves to a Bundle),
  ADR-068 (Phase 6 PR slicing)

## Context

Phase 4 PR 4 (ADR-037) shipped the Slang shader toolchain. It produces
`Bundle` artefacts (`tools/engine-shader/src/artifact.rs`): per-target
compiled bytecode (SPIR-V, WGSL, DXIL, MSL), reflection JSON, BLAKE3
digest. The engine-asset pak (ADR-008) can store them and the runtime
can decode them via `impl Asset for Bundle`.

Phase 5 PR 2 (ADR-049) shipped `engine-gpu` with `RenderPipeline` and
`ComputePipeline` types that wrap `wgpu::RenderPipeline` /
`wgpu::ComputePipeline`. Their constructors take the bytecode, the
vertex/fragment entry point names, the bind-group layout descriptors,
and the pipeline state (rasterizer, depth, blend).

The gap, today, is that no engine code wires the two together. The
pass `record()` bodies in `crates/engine-render/src/passes.rs` are
no-ops (per `docs/architecture/engine-render.md:41` — "GPU-pass
`record()` is a Phase-6 deliverable"). Phase 6 PR 2 closes that gap
with a *binding layer*: a small set of helper functions that take a
loaded Slang `Bundle` + a render-graph pass type + a target enum and
produce a ready-to-bind `engine_gpu::RenderPipeline` or
`ComputePipeline`.

This is the first PR where the renderer compiles a real shader against
a real wgpu device end-to-end. It does so without leaking `wgpu::*`
identifiers across the ADR-049 boundary — the helpers consume
`engine_gpu::Device` and emit `engine_gpu::RenderPipeline`.

## Decision

### 1. New module `engine_render::shader`

`crates/engine-render/src/shader.rs` (new module, exported from
`lib.rs`):

```rust
pub struct ShaderArtefactSet {
    bundle: engine_asset::Handle<engine_shader::Bundle>,
}

impl ShaderArtefactSet {
    pub fn new(bundle: Handle<Bundle>) -> Self;

    /// Pick the artefact for the engine_gpu device's preferred backend
    /// (SPIR-V for Vulkan, WGSL for WebGPU/native default, DXIL for
    /// D3D12, MSL for Metal).
    pub fn for_device(&self, device: &engine_gpu::Device)
        -> Result<&engine_shader::Artifact, ShaderError>;

    /// Reflection-driven bind-group layout extraction. Reads the
    /// Slang reflection JSON in the artefact; emits an
    /// engine_gpu::BindGroupLayoutDesc.
    pub fn bind_group_layouts(&self, device: &engine_gpu::Device)
        -> Result<Vec<BindGroupLayoutDesc>, ShaderError>;
}
```

`ShaderArtefactSet` is also a `ResourceType` in the render graph
(`engine_render::resources::ShaderArtefactSet`) — passes that need
shaders declare them in `reads()` and the graph resolves them at
compile time.

### 2. Helper: `build_render_pipeline`

```rust
pub fn build_render_pipeline(
    device: &engine_gpu::Device,
    artefacts: &ShaderArtefactSet,
    desc: &RenderPipelineDesc,
) -> Result<engine_gpu::RenderPipeline, ShaderError>;

pub struct RenderPipelineDesc<'a> {
    pub label: &'static str,
    pub vertex_entry: &'a str,    // typically "vs_main"
    pub fragment_entry: Option<&'a str>, // typically "fs_main"
    pub vertex_layouts: &'a [VertexBufferLayout<'a>],
    pub color_targets: &'a [ColorTarget],
    pub depth_stencil: Option<DepthStencilState>,
    pub primitive: PrimitiveState,
    pub multisample: MultisampleState,
}
```

`VertexBufferLayout`, `ColorTarget`, `DepthStencilState`,
`PrimitiveState`, and `MultisampleState` are owned engine-render types
that translate to the equivalent `engine_gpu::*` and ultimately
`wgpu::*` configurations.

### 3. Helper: `build_compute_pipeline`

```rust
pub fn build_compute_pipeline(
    device: &engine_gpu::Device,
    artefacts: &ShaderArtefactSet,
    desc: &ComputePipelineDesc,
) -> Result<engine_gpu::ComputePipeline, ShaderError>;

pub struct ComputePipelineDesc<'a> {
    pub label: &'static str,
    pub entry: &'a str,    // typically "cs_main"
    pub workgroup_size: [u32; 3],   // ground truth; reflection cross-check
}
```

### 4. Bind-group slot allocation convention

The reflection JSON drives a deterministic slot assignment:

- **Group 0:** per-frame uniforms (view matrix, projection, jitter,
  frame index, time). Bind once per frame; shared by every pass.
- **Group 1:** per-pass uniforms (shadow cascade splits, cluster grid
  parameters, post-FX constants). Bind at pass `record()` entry.
- **Group 2:** per-material uniforms + textures (driven by EMAT,
  ADR-061). Bind per-draw via the bindless heap (ADR-044) when
  feasible; fall back to a per-material BindGroup for shaders that
  declare non-bindless textures.
- **Group 3:** per-draw uniforms (model matrix, sub-mesh material
  index, instance ID). Pushed via push-constants where the device
  supports 128 B; falls back to a tiny BindGroup otherwise.

The reflection cross-check (`ShaderArtefactSet::bind_group_layouts`)
asserts that the bundled Slang artefact declared groups 0–3 in this
order; a mismatch is `ShaderError::ReflectionGroupOrder`. This makes
the convention machine-checked, not merely documentary.

### 5. Push-constant budget

Bottom-tier hardware (RX 580 milestone) supports 128 B push-constants
on Vulkan, 128 B on Metal, 256 B on D3D12. The engine uses **64 B
maximum** per pass, leaving headroom and matching WebGPU's eventual
push-constant cap (the proposal pins 64 B as of 2026). Layout:

```text
PushConstants (64 B):
  model_xform     [f32; 12]    // 3x4 affine (48 B)
  material_index  u32           // index into EMAT bindless pool (4 B)
  instance_id     u32           // for indirect draws (4 B)
  flags           u32           // per-draw bitset (4 B)
  reserved        u32           // pad (4 B)
```

Passes that need more per-draw state use Group 3.

### 6. Pipeline cache

`engine_render::shader::PipelineCache` deduplicates pipelines by
`(shader_artefact_hash, RenderPipelineDesc-hashed)`. The cache is
process-local (no on-disk persistence in PR 2); a future ADR may
add a pak-cached form. The cache uses the owned `engine_core::
collections::HashMap` with `DeterministicHasher` (ADR-028) so
iteration order is stable across runs — relevant for the cross-arch
determinism oracle.

## Rationale

- **The shader artefact already exists.** Phase 4 produced the
  `Bundle` format; Phase 5 produced `engine_gpu::RenderPipeline`.
  Phase 6 PR 2's contribution is to *connect* them without leaking
  either side's types past the engine-render boundary.
- **Reflection-driven bind-group layout is mandatory for bindless.**
  ADR-044's heap requires bindless-array binding; the layouts must
  match the shader's declaration exactly; the source of truth is
  the reflection JSON the toolchain already produces.
- **Fixed group ordering simplifies cross-pass binding.** Twenty
  passes don't each invent their own group layout — they consume the
  same per-frame uniforms in group 0, the same per-material textures
  via bindless in group 2.
- **Push-constants at 64 B work on every backend.** Avoids per-pass
  BindGroup creation for the common case (`model_xform` +
  `material_index`).
- **The pipeline cache is a perf necessity.** Recompiling a 20-pass
  pipeline-state-object on every frame is several ms; the cache is
  small and unbounded (pipelines are long-lived).

## Consequences

- `engine-render` gains one new module (`shader`) and one new
  resource type (`ShaderArtefactSet`). No new dependencies.
- The 11 Phase-5 pass stubs (`CullPass`, `CsmShadowPass`, …,
  `TonemapPass`, `UpscalePass`) will, in PR 3 + PR 4, gain a
  `pipeline: engine_gpu::RenderPipeline` (or `ComputePipeline`)
  field initialized at first `record()` entry via the helpers
  from this ADR.
- The render graph's `compile()` step (ADR-039) now allocates
  per-pass `ShaderArtefactSet` references from the graph's
  resource pool; pipeline construction is lazy at first
  `execute()`.
- Slang shader sources move into `crates/engine-render/shaders/`
  (a new directory). The pre-build script invokes
  `tools/engine-shader/` to produce the `.bundle` artefacts; the
  runtime loads them through the asset pipeline.
- The reflection cross-check failure mode is a new build-time
  diagnostic; documented in the `ShaderError` enum's doc-comment.

## Risks and tradeoffs

- **Reflection drift between Slang artefact and helper code.** If
  the shader's bind-group layout changes without updating the helper's
  expected layout, `ShaderError::ReflectionGroupOrder` fires at
  startup. Mitigation: the layout is asserted in unit tests against
  every shipped shader's artefact, not deferred to runtime.
- **Push-constant 64 B limit is real.** Shaders that need >64 B per
  draw must use Group 3 instead. Documented in the shader-author
  conventions in `crates/engine-render/shaders/README.md`.
- **The pipeline cache is process-local.** First frame after engine
  start compiles all pipelines — observable as a "first frame stall".
  Mitigation: PR 6's frame-pacing measurement excludes frame 0 from
  p99/σ stats; the warm-up frame is reported separately.
- **No async pipeline compilation in PR 2.** wgpu's
  `create_render_pipeline` is synchronous; multi-second
  multi-pipeline init is conceivable on cold cache. Mitigation: the
  pipeline cache populates lazily per-pass, so unused passes pay
  nothing. Async compile is a Phase 7+ optimization.

## Alternatives considered

- **Author bind-group layouts in Rust, ignore reflection.** Loses the
  cross-check; a shader edit can silently desync from runtime
  expectations. Rejected.
- **Push-constants 128 B (full Vulkan budget).** Loses WebGPU
  compatibility per the WebGPU spec's eventual cap. Rejected for
  cross-target portability.
- **One global BindGroup per pass instead of group-0/1/2/3
  hierarchy.** Simpler authoring; loses per-frame uniform sharing
  across passes (a major perf miss on a 10-pass renderer). Rejected.
- **Pak-cached pipelines (PSO blob storage).** Eliminates first-frame
  stall; couples the cache to a specific GPU + driver combination;
  the cache must be invalidated per driver update. Phase 7+
  candidate.
- **A bigger helper trait (`Pass::pipeline_desc()`)** instead of a
  free function. Tighter coupling between Pass and pipeline shape;
  less reusable across pass types that share a pipeline.
  Rejected — free functions are the smaller surface.

## Verification

- Implementation lands in Phase 6 PR 2. Test files:
  - `crates/engine-render/src/shader.rs` doc-tests for
    `ShaderArtefactSet::for_device` target selection.
  - `crates/engine-render/tests/pipeline_smoke.rs`: load a Slang
    bundle for the `pbr_opaque` shader + the `clear_blit` shader;
    construct a `RenderPipeline` and a `ComputePipeline`; run a
    minimal pass on a headless `engine_gpu::Device`; capture a
    deterministic 256×256 RGBA output; compare against a baked
    golden image.
  - `crates/engine-render/tests/reflection_layout.rs`: for every
    shipped shader, assert the reflection JSON's bind-group order
    matches the engine's group 0/1/2/3 convention; fail with the
    shader name + group index on mismatch.
- CI: existing wgpu boundary guard (ADR-049) covers; new helpers
  consume only `engine_gpu::*`.
- Telemetry: `SPAN "render.pipeline.compile"` + `COUNTER
  "render.pipeline.cache_miss"` (ADR-010).
- Phase 6 PR 3 + PR 4 are the first consumers of the helpers
  introduced here; their pixel-parity oracles are the integration
  verification.
