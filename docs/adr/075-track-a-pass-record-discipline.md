# ADR-075 — Track-A pass `record()` implementation discipline

- Status: Accepted
- Date: 2026-05-27
- Phase: 5.5 — Track A GPU binding closure (per ADR-069 reconciliation)
- Companion: ADR-039 (render graph), ADR-044 (bindless texture heap),
  ADR-049 (engine-gpu wgpu wrapper boundary), ADR-064 (geometry +
  lighting GPU pass contracts), ADR-065 (post-FX GPU pass contracts),
  ADR-068 (Phase 6 PR slicing — superseded by ADR-069's renumbering),
  ADR-069 (engine-vs-spec phase reconciliation), ADR-074 (wgpu Vulkan
  backend activation)

## References

- *Game Engine Architecture*, 3rd ed. (Gregory, 2018), Ch. 11.5
  ("Render Queue") + Ch. 8 ("Parallelism") — the command-buffer model
  and per-frame state separation this ADR encodes.
- *Vulkan Programming Guide* (Sellers / Kessenich, 2017), Ch. 6
  ("Descriptor Sets" — the abstraction wgpu's bind groups wrap), Ch. 9
  ("Compute"), Ch. 13 ("Multipass Rendering" — attachment management).
- *Real-Time Rendering*, 4th ed. (Akenine-Möller / Haines / Hoffman,
  2018), Ch. 18.4 ("Bindless / Indexed-Array Rendering") — the
  motivation for ADR-044's bindless heap and the per-frame draw-call
  cost model that bind-group caching protects.

## Context

Phase 5.5's A.1 (ADR-074) enabled the wgpu Vulkan backend; the
`device_init_against_real_adapter` smoke proves wgpu reaches the user's
RX 580. The 11 Track-A pass `record()` bodies under
`crates/engine-render/src/passes.rs` are now the gating gap: 7 compute
passes issue `dispatch_workgroups(1, 1, 1)` placeholders against empty
bind-group layouts; 3 render passes have empty `record()` bodies
because `begin_render_pass` needs attachment views the render graph's
transient pool does not yet resolve.

The `build_all_phase6_pipelines_against_real_device` smoke
(`#[ignore]`'d pending this ADR) correctly fails today on the first
pass with "binding missing from pipeline layout" — wgpu's validation
catches the mismatch between the WGSL `@group(0) @binding(0)`
declarations and the Rust-side empty `BindGroupLayout`.

Without a documented discipline, each pass implementer might choose
differently — inlining bind-group construction here, factoring it
there, plumbing attachment views through one indirection or another,
caching per-frame vs per-call — and the code surface would fragment.
This ADR is the per-pass implementation contract every Track-A pass
honours.

## Decision

### 1. `record()` body template (six steps, in order)

Every Track-A `Pass::record` body follows this exact shape:

```rust
fn record(&mut self, ctx: &mut PassContext) {
    // Step 1. CPU-oracle short-circuit. When `ctx.gpu` is `None`
    //         (the rasterizer testbed path), the pass returns
    //         immediately — the CPU oracle owns the equivalent work
    //         in `testbed/engine-raster/src/*`.
    let Some(gpu) = ctx.gpu.as_mut() else { return; };

    // Step 2. Pipeline-installed short-circuit. When the pass has
    //         been registered to a scheduling-only graph (no
    //         `install_pipelines` call), the optional pipeline is
    //         `None` and the pass returns. Preserves the PR-7.5
    //         no-op-on-missing-pipeline behaviour.
    let Some(pipeline) = self.pipeline.as_ref() else { return; };

    // Step 3. Resolve resource views from the graph's transient
    //         pool via `ctx.resources` (the resolver added in this
    //         ADR's render-graph extension). Static textures come
    //         from the bindless heap via `ctx.bindless`.
    let views = bindgroups::cull::Views::resolve(ctx, self)?;

    // Step 4. Build the bind group(s) from the resolved views +
    //         per-frame uniforms via the pass's bindgroups module.
    //         Bind groups are constructed each frame; the
    //         constructor is in `bindgroups/<pass>.rs`.
    let bind_group = bindgroups::cull::build(gpu.device, &views, /*uniforms*/);

    // Step 5. Open the pass scope and issue real GPU work. Dispatch
    //         counts come from the resolved views (e.g. SSBO
    //         element count divided by workgroup size); draw counts
    //         come from indirect buffers the cull pass produced.
    let mut cpass = gpu.encoder.begin_compute_pass(self.name());
    cpass.set_pipeline(pipeline);
    cpass.set_bind_group(0, &bind_group, &[]);
    cpass.dispatch_workgroups(real_count_x, real_count_y, real_count_z);

    // Step 6. End-of-scope drops the pass scope; the encoder is
    //         owned by `RenderGraph::execute` and submitted once
    //         per frame at the encoder boundary.
}
```

Render passes follow the same template, swapping
`begin_compute_pass` for `begin_render_pass` (with attachment views
from Step 3) and `dispatch_workgroups` for `draw_indexed_indirect` (or
`draw(0..3, 0..1)` for full-screen passes).

### 2. `crates/engine-render/src/bindgroups/` module organisation

A new directory with one file per pass:

```
crates/engine-render/src/bindgroups/
├── mod.rs           # `pub mod cull; pub mod csm_shadow; ...`
├── cull.rs          # `pub struct Views { ... } pub fn build(...) -> BindGroup`
├── csm_shadow.rs
├── cluster_assign.rs
├── gbuffer.rs
├── lighting.rs
├── ssao.rs
├── ibl_evaluate.rs
├── taa_resolve.rs
├── bloom.rs         # owns extract / downsample / upsample triple
├── tonemap.rs
└── brdf_lut_bake.rs # init-time, called from `crate::init`
```

Each file exposes:

- A `Views` struct that names every resource the pass consumes (color
  attachments, depth attachments, storage buffers, samplers).
- A `resolve(ctx: &PassContext, pass: &PassStruct) -> Self::Views`
  associated function that looks up each `ResourceId` via the graph's
  resolver and returns the typed view collection.
- A `build(device, &views, &uniforms) -> BindGroup` function that
  constructs the bind group against the pass's layout.
- A `layout(device) -> BindGroupLayout` function that builds the
  layout from a `BindGroupLayoutDescriptor`. Called once during
  pipeline construction; cached on the pass struct.

The bind-group layout structure mirrors the WGSL `@group/@binding`
declarations 1:1. A discrepancy (Rust says binding 3 is a uniform
buffer, WGSL says it's a storage buffer) is caught by wgpu's
pipeline-creation validation at startup — the
`build_all_phase6_pipelines_against_real_device` smoke is the gate.

### 3. WGSL `@group/@binding` is the source of truth

The WGSL shader sources under `crates/engine-render/shaders/*.wgsl` are
the authoritative declarations. Rust-side `BindGroupLayoutDescriptor`
constructors mirror them. If they drift, the smoke test catches the
mismatch. The ADR-068 close addenda planned a Naga-reflection-based
auto-derive of the Rust layout from the WGSL — this is deferred to a
Phase 6+ tooling improvement; for Phase 5.5 the hand-mirroring is
acceptable because (a) the bindings are small (≤ 6 per pass), (b) the
smoke test is fast feedback, (c) the WGSL declarations are stable.

### 4. Pipeline construction (install-once) vs bind-group construction (per-frame)

- **Pipelines** are installed once per device session via
  `Pass::install_pipeline` (called by
  `RenderGraph::install_pipelines` at startup, per PR 7.5). The
  bind-group *layout* is built at the same time; the layout is what
  the pipeline references via its `PipelineLayoutDescriptor`.
- **Bind groups** are built per frame in `record()`. The underlying
  views change frame-to-frame (swapchain rotation, TAA history
  ping-pong, transient-pool reuse), so the bind group can't be
  cached at install time. A future optimisation (Phase 6+) may cache
  bind groups keyed by `(view-identity-tuple)`; for Phase 5.5 the
  per-frame build is fine — the 10 active passes × ~30 µs each ≈
  300 µs of CPU per frame, well within the 16.6 ms frame budget at
  60 FPS on i7-6700.

### 5. Attachment-view plumbing through the render graph

`RenderGraph::compile` gains a second phase: after the topological
sort, allocate transient resources (color attachments, depth
attachments, SSBOs) from the device's transient pool. The pool
returns a `ResourceTable` keyed by `ResourceId` that maps to typed
view handles.

`PassContext` gains:

```rust
pub struct PassContext<'a> {
    pub frame_idx: u64,
    pub gpu: Option<GpuFrameContext<'a>>,
    pub resources: Option<&'a dyn ResourceResolver>,
    pub user: &'a mut dyn core::any::Any,
}

pub trait ResourceResolver {
    fn resolve_view(&self, id: ResourceId)   -> Option<&engine_gpu::TextureView>;
    fn resolve_buffer(&self, id: ResourceId) -> Option<&engine_gpu::Buffer>;
    fn resolve_sampler(&self, id: ResourceId) -> Option<&engine_gpu::Sampler>;
}
```

The resolver lives in `crates/engine-render/src/render_graph/views.rs`.
`RenderGraph::execute` constructs it from the per-frame transient pool
allocation and passes it through `PassContext::resources`. Passes that
need persistent resources (the bindless heap, the BRDF LUT cached at
init) consult separate accessors on `PassContext` (added as needed).

CPU oracle implementers (`testbed/engine-raster`) pass
`resources: None` since their world state lives in `ctx.user`.

### 6. Vertex / index buffer binding for render passes

Render passes that draw mesh geometry (`GBufferPass`,
`CsmShadowPass`) consume the `IndirectDrawBuffer` the `CullPass`
produces. The mesh asset (EMSH per ADR-061) is loaded into a
device-side vertex + index buffer pair held by the world state
(`ctx.user`). The render pass downcasts `ctx.user` to its expected
world type, retrieves the buffer handles, and binds them before
issuing `draw_indexed_indirect(buffer, offset, draw_count_max)`.

Full-screen passes (`LightingAccumulationPass`) use no vertex buffer:
`vertex_buffers: &[]` in the existing pipeline desc; the WGSL vertex
shader generates a 3-vertex triangle covering NDC via
`@builtin(vertex_index)`. The pass calls `draw(0..3, 0..1)`.

The mesh-buffer ownership pattern (world state holds the buffers, the
pass dereferences them through `ctx.user`) keeps the render graph free
of per-asset state — a Gregory Ch. 11.5 design point.

### 7. CPU oracle parity guarantee

Every Track-A pass has a CPU-side reference in `testbed/engine-raster`
(per ADR-046 + the existing `combined_deferred_scene` test). The
`ctx.gpu == None` short-circuit (Step 1) is the boundary: the GPU
path takes the new template; the CPU path is untouched. The
pixel-parity oracle (ADR-046, this plan's A.3) compares the two
end-to-end.

### 8. A.2 sub-PR plan (revised after A.2a landed)

The A.2 work splits into:

- **A.2a — auto-derive pipeline bootstrap (landed 2026-05-27).** Took
  a shorter path than the original A.2a plan above: instead of
  authoring 11 `bindgroups/` modules upfront, the `engine-gpu`
  pipeline descriptors gained an `Option<&PipelineLayout>` layout
  field, with `None` selecting wgpu's auto-derive (wgpu introspects
  the WGSL `@group/@binding`/`var<immediate>` declarations and
  synthesises the layout). `crates/engine-render/src/shader.rs`'s
  `build_compute_pipeline` and `build_render_pipeline` helpers pass
  `None`, so all 11 + BRDF LUT pipelines construct cleanly via
  reflection. The auto-derived layout is queryable per-set via
  `RenderPipeline::bind_group_layout(set_index)` /
  `ComputePipeline::bind_group_layout(set_index)`.

  A.2a also surfaced and resolved wgpu-29 / Naga-29 migration items
  that the original PR-7 shaders pre-dated: `var<push_constant>` →
  `var<immediate>`; `bgra8unorm_srgb` storage format → `bgra8unorm`
  storage format with manual linear→sRGB encoding in the tonemap
  shader (storage-texture writes bypass swapchain view sRGB
  conversion); `engine-gpu::DeviceFeatures` gained `multiview` (CSM
  `@builtin(view_index)` needs `VK_KHR_multiview`),
  `adapter_specific_format_features` (R16Float storage write), and
  `bgra8unorm_storage` (the tonemap swapchain-compatible output).
  Mesh-vertex buffer layout (12 + 12 + 16 + 8 = 48 bytes per ADR-061)
  added to shadow + gbuffer pipelines so the shader-input contract
  matches the EMSH binary layout.

  `build_all_phase6_pipelines_against_real_device` now passes in the
  default workspace gate. The implicit bind-group layouts are queryable
  for the record() bodies in A.2b.

- **A.2b — resource resolver + record() body wiring.** The
  `crates/engine-render/src/render_graph/views.rs` `ResourceResolver`
  trait + the per-frame transient-pool allocator in
  `RenderGraph::execute` + `PassContext.resources` field +
  bind-group construction in each pass's `record()` body using the
  pipeline's auto-derived layout (queried via
  `pipeline.bind_group_layout(group_idx)`) + the 3 render-pass bodies
  wired with `begin_render_pass` + vertex/index/draw-indirect
  bindings + `init::bake_brdf_lut(device, queue) ->
  engine_gpu::Texture` one-shot startup helper. Closes the dispatch
  half.

- **A.2c (optional, post-v0.3) — explicit `bindgroups/` modules per
  pass.** The principled long-term home for the layouts; replaces
  auto-derive on a per-pass basis. Lands as cleanup either alongside
  A.3 (pixel-parity fixtures) or as a dedicated PR. Not required for
  Engine Core v0.3.

## Rationale

- **A single template prevents fragmentation.** Without an ADR, each
  pass implementer would choose differently for "where bind-group
  construction lives", "how attachment views are plumbed", etc. The
  six-step template is the canonical answer.
- **`bindgroups/` is one-file-per-pass.** Mirrors `shaders/` (one
  WGSL per pass). Locality of reference: a future engineer reading
  `passes.rs::CullPass` knows to read `bindgroups/cull.rs` next, and
  the WGSL `shaders/cull.wgsl` third. Three files, one pass.
- **Per-frame bind groups + install-once pipelines** matches wgpu's
  cost model. Pipeline construction is expensive (SPIR-V cross-compile
  via Naga, driver-side compilation); bind-group construction is
  cheap (a `Vec<BindGroupEntry>` build + a wgpu call). The split is
  Gregory's "preprocess once vs. submit-each-frame" discipline.
- **`ResourceResolver` is a trait, not a concrete struct.** Lets the
  CPU oracle pass `None`; lets future test harnesses inject mock
  resolvers; lets the GPU runtime own the transient pool without
  the trait knowing.
- **WGSL is the source of truth.** A future tooling improvement
  (Naga reflection → autogenerated Rust layouts) is welcome but is
  not the Phase 5.5 critical path. Hand-mirroring with smoke-test
  validation is the right balance now.
- **CPU oracle parity is non-negotiable.** ADR-046 names the oracle
  as the contract; the `ctx.gpu == None` short-circuit is the
  mechanism. Every pass honours it.

## Consequences

- 11 new files under `crates/engine-render/src/bindgroups/` (one per
  pass + the BRDF LUT bake).
- 1 new file `crates/engine-render/src/render_graph/views.rs` with
  the `ResourceResolver` trait + the default `TransientResourceTable`
  implementation.
- `PassContext` gains a `resources: Option<&dyn ResourceResolver>`
  field. Existing test stubs in `crates/engine-render/src/render_graph.rs`
  (the `Producer` / `Consumer` test passes) add `resources: None` to
  one call site.
- `RenderGraph::execute` becomes two-phase: transient pool allocation
  before the pass loop, resolver passed through the per-iteration
  `PassContext`.
- All 11 pass `record()` bodies are rewritten per the six-step
  template. `passes.rs` roughly doubles in length (1189 → ~2200 lines
  estimated), but the template is uniform — each pass is 60–120
  additional lines of bind-group + view-resolution code.
- `crates/engine-render/src/init.rs::bake_brdf_lut(device, queue) ->
  engine_gpu::Texture` lands as the one-shot init helper.
- The full-pipeline smoke test removes its `#[ignore]` once A.2b
  lands. `pipeline_smoke` joins the workspace gate.

## Risks and tradeoffs

- **`PassContext` signature change is a breaking change** for any
  out-of-tree `Pass` implementer. None exist today; the test stubs
  are the only call sites. Risk is bounded.
- **Per-frame bind-group construction has CPU cost.** Bounded
  measurement: ~30 µs per pass on Skylake i7-6700 × 10 active passes
  = ~300 µs per frame. Within the 16.6 ms budget. Phase 6+ may cache
  bind groups by view-identity tuple if measurement shows it matters.
- **Naga reflection auto-derive is deferred.** The hand-mirroring of
  Rust layout vs WGSL `@group/@binding` is a hand-maintained
  invariant. The smoke test is the safety net: any drift fails
  loudly at startup. Phase 6+ tooling can automate this; for Phase
  5.5 it's not the bottleneck.
- **CPU oracle has no `record()` per ADR-046**; it has its own
  `combined_deferred_scene` driver in `testbed/engine-raster`. The
  `ctx.gpu == None` short-circuit covers the case where a GPU-path
  pass is registered in a CPU-only graph (the testbed). The parity
  oracle is an end-to-end test, not a per-pass test.

## Alternatives considered

- **Inline bind-group construction in `passes.rs`.** Rejected —
  doubles the noise in `passes.rs` and dilutes the orchestration
  logic with descriptor authoring.
- **One `bindgroups.rs` file with all passes' layouts.** Rejected —
  the file would be ~1500 lines of unrelated layouts; locality of
  reference suffers.
- **Generate bind-group layouts from WGSL via Naga at build time.**
  Tempting — Naga has a reflection API. Rejected for Phase 5.5:
  the auto-derive needs build.rs integration, the Naga reflection
  surface for wgpu's `BindGroupLayoutDescriptor` is not 1:1, and
  Phase 5.5's priority is the milestone, not tooling. Phase 6+
  candidate.
- **Cache bind groups by view-identity tuple.** Optimisation. Not
  necessary at Phase 5.5 measurement; revisit if `just frame-pacing`
  shows bind-group construction in the top-N cost contributors.
- **Use wgpu's "auto" pipeline layout** (`device.create_pipeline_layout`
  with `bind_group_layouts: &[]` + auto-derive from shader source).
  wgpu supports this for prototypes; it's not the right answer for
  production because (a) it can't express dynamic-offset uniforms,
  (b) it can't express static samplers, (c) the layout becomes opaque
  to the engine.

## Verification

- The six-step template is documented above; a one-time grep test
  (`crates/engine-render/tests/pass_record_discipline.rs`, lands with
  A.2a) asserts every `record()` body in `passes.rs` follows the
  pattern (regex on the source).
- `build_all_phase6_pipelines_against_real_device` removes its
  `#[ignore]` when A.2b lands. The smoke is the contract — any
  Rust-WGSL layout mismatch fails the workspace gate.
- `crates/engine-render/tests/pipeline_smoke.rs`
  `device_init_against_real_adapter` (ADR-074) continues to pass.
- The 6 pixel-parity oracle fixtures (this plan's A.3) exercise the
  full pipeline end-to-end on the user's RX 580.
- `cargo test --workspace` end count after A.2 closes: ~635 (615
  baseline + ADR-074's +1 + A.2's bind-group authoring tests + A.3's
  parity fixtures).
- `radeontop` shows the RX 580 GPU utilisation during the parity
  fixtures.

## Pre-merge engineering checklist

- [x] ADR drafted with template + module organisation.
- [ ] A.2a: `bindgroups/` skeleton + 7 compute pass bodies + the
      pass-record-discipline grep test.
- [ ] A.2b: render-graph transient-pool allocator + 3 render pass
      bodies + `init::bake_brdf_lut` + remove the smoke `#[ignore]`.
- [ ] A.3: 6 pixel-parity oracle fixtures.
