# ADR-083 — `UpscalePass::record()` wiring discipline + in-tree EASU shader

- Status: Accepted (Phase 6 design contract; implementation lands in
  Phase 6 PR 1a)
- Date: 2026-05-28
- Phase: 6 — NEURAL RENDERING & GAUSSIAN SPLATTING
- Companion: ADR-005 (vendor upscaler trait), ADR-039 (render graph),
  ADR-049 (engine-gpu wrapper), ADR-063 (shader artefact to pipeline),
  ADR-066 (vendor upscaler binding discipline), ADR-067 (Owned ONNX
  temporal upscaler), ADR-068 (prior Phase-6 slicing — PR 7 / PR 8
  addendum that documented the current no-op),
  ADR-075 (Track-A pass record() discipline), ADR-076 (FSR EASU
  spatial fallback — this ADR realises its step 2),
  ADR-084 (Phase 6 PR slicing)

## Context

Phase 5.5 closed with `UpscalePass::record()` as a documented no-op:

```rust
// crates/engine-render/src/passes.rs:1999 (Phase 5.5)
fn record(&mut self, _ctx: &mut PassContext) {
    // PR 7: no-op. The upscaler dispatches through
    // [`crate::upscale::UpscalerRegistry`] which is not yet
    // threaded through `PassContext`; PR 8 wires the registry
    // lookup + `provider.upscale(&mut UpscaleCtx { .. })` call.
    // CPU oracle reference: `engine_raster::upscale::bilinear_upscale`.
}
```

The Phase 5.5 PR 7 / PR 8 addendum on ADR-068 documented this gap.
`VendorFsr::supports() == true` (ADR-076) and
`OwnedOnnxTemporal::supports() == true` (Phase 5.5 A.4 / A.5
commit `08f6bd9`) currently return *token-only* `UpscaleResult`s;
neither actually upscales pixels. The cascade *select* is correct
(`vendor.fsr` wins on the user's RX 580); the cascade *dispatch* is
a no-op.

ADR-076 step 2 named an in-tree WGSL EASU shader as the FSR
spatial-fallback's runtime; that shader (`fsr_easu.wgsl`) has not
yet landed.

Phase 6 closes both gaps:

1. `UpscalePass::record()` becomes a real body that dispatches
   through the `UpscalerRegistry` resolved from `PassContext`.
2. `crates/engine-render/shaders/fsr_easu.wgsl` lands in tree as
   the FSR-EASU runtime, realising ADR-076 step 2.
3. `crates/engine-render/shaders/bilinear_upscale.wgsl` lands as
   the OwnedBilinear runtime — replacing the documented
   CPU-oracle delegation with a real GPU dispatch so the cascade's
   final fallback is itself GPU-resident.

## Decision

### 1. `PassContext::upscaler` field

`crates/engine-render/src/render_graph.rs` — `PassContext` grows
an optional reference to the upscaler registry:

```rust
pub struct PassContext<'a> {
    pub gpu: &'a engine_gpu::Device,
    pub encoder: &'a mut engine_gpu::Encoder,
    pub frame_idx: u64,
    pub resources: &'a TransientResourceTable,
    pub upscaler: Option<&'a UpscalerRegistry>, // new in Phase 6 PR 1a
}
```

`RenderGraph::execute()` threads the registry through. The field
is `Option<>` so passes that don't need an upscaler don't carry
the field's invariants.

### 2. `UpscalePass::record()` body

```rust
// crates/engine-render/src/passes.rs

fn record(&mut self, ctx: &mut PassContext<'_>) {
    let Some(registry) = ctx.upscaler else {
        // No registry attached: skip the upscale (the resolved
        // target is the final target). Log via the existing
        // SelectionLogger callback.
        return;
    };
    let provider = registry.select(ctx.gpu);

    let resolved_view = ctx.resources.view(self.resolved);
    let upscaled_view = ctx.resources.view(self.upscaled);

    let mut upscale_ctx = UpscaleCtx {
        gpu: ctx.gpu,
        encoder: ctx.encoder,
        color: &resolved_view,
        target: &upscaled_view,
        motion_vectors: ctx.resources.try_view(self.motion_vectors),
        depth: ctx.resources.try_view(self.depth_full),
        frame_idx: ctx.frame_idx,
        jitter: self.jitter,
    };

    match provider.upscale(&mut upscale_ctx) {
        Ok(_result) => { /* provider has written upscaled_view */ }
        Err(err) => {
            // Provider failed mid-frame; the cascade's fallback is
            // OwnedBilinear which is the always-supported floor.
            // Log + record the downgrade for telemetry.
            registry.log_runtime_downgrade(provider.kind(), err);
            // OwnedBilinear is always supported; dispatch directly.
            let bilinear = registry.owned_bilinear();
            let _ = bilinear.upscale(&mut upscale_ctx);
        }
    }
}
```

The fallback to `OwnedBilinear` on runtime error is the
"degrade-but-keep-frames" property ADR-067 § names: the user
always gets a frame, even if a vendor SDK crashes mid-dispatch.

### 3. `fsr_easu.wgsl` — in-tree EASU + RCAS

A new compute shader at
`crates/engine-render/shaders/fsr_easu.wgsl` implements GPUOpen
FidelityFX FSR 1.0's EASU edge-adaptive spatial upsampling +
optional RCAS contrast-adaptive sharpening pass. The shader is a
direct WGSL port of the published HLSL reference
(`https://github.com/GPUOpen-Effects/FidelityFX-FSR/tree/v1.1`,
MIT licensed).

Polaris-compatibility constraints (matters because the user's
RX 580 is Polaris GFX8):

- No subgroup intrinsics (`subgroupShuffle`, etc.).
- No `f16` arithmetic; pure `f32` throughout.
- Workgroup size (8, 8, 1).
- One `@compute` entry point: `easu_main`. Optional RCAS pass
  is a second compute pipeline (`rcas_main`) bound separately.

`VendorFsr::upscale()` (in `crates/engine-render/src/upscale.rs`)
becomes a real EASU dispatch — replacing the token-return stub
from Phase 5.5 — when the `fsr` cargo feature is *off* (the
in-tree EASU path; ADR-076's default). When `fsr` is *on*, the
tensor-accelerated FSR 4 path from ADR-079 takes precedence (and
falls back to EASU when the device lacks RDNA-4 tensor support).

### 4. `bilinear_upscale.wgsl` — real OwnedBilinear

A second new compute shader at
`crates/engine-render/shaders/bilinear_upscale.wgsl` implements a
straightforward `textureSampleLevel` 2× bilinear upsample.
Workgroup (8, 8, 1); minimal LOC (~50). `OwnedBilinear::upscale()`
dispatches this instead of delegating to the CPU oracle.

This matters because:

- The cascade's *final* fallback is now self-contained on GPU.
- The CPU oracle (`engine_raster::upscale::bilinear_upscale`) was
  test reference, not a production path; removing the runtime
  call-through tightens the production / oracle split.
- The Phase 5.5 PR 7 / PR 8 addendum's documented "CPU oracle
  delegation" stops being load-bearing on the production path.

### 5. Dispatch verification test

A new test at `crates/engine-render/tests/upscale_dispatch.rs`
exercises `UpscalePass::record()` end-to-end against the user's
RX 580 via the Mesa RADV path enabled in ADR-074:

```rust
#[test]
fn upscale_pass_dispatches_bilinear() {
    // 16×16 → 32×32 OwnedBilinear; assert non-trivial output
    // (not zero, not pass-through identity).
}

#[test]
fn upscale_pass_dispatches_fsr_easu() {
    // 16×16 → 32×32 VendorFsr (EASU spatial path);
    // assert non-trivial output + assert max_delta vs
    // OwnedBilinear is non-zero (EASU is edge-adaptive,
    // not bilinear).
}
```

The test runs in the existing engine-render integration-test
harness (not behind a feature flag — both paths are default-build
reachable). Skipped gracefully if no compute-capable adapter is
present at test time.

### 6. Cascade-runtime telemetry

The bench JSON report (`bin/engine-bench-frame-pacing`) gains a
field:

```json
"upscale_dispatch_ms_p99": 0.42,
```

so the per-frame upscale cost is observable independent of
selected-provider drift.

### 7. CPU oracle stays for parity tests

`engine_raster::upscale::bilinear_upscale` (the CPU reference) is
preserved as the parity oracle for the existing
`upscale_bilinear` pixel-parity fixture (Phase 5 PR 5). The fixture
compares the new GPU `bilinear_upscale.wgsl` against the CPU
reference at strict 1/255 — a *new* parity assertion that lands
in PR 1a as part of the dispatch wiring.

## Consequences

### Positive

- `VendorFsr` + `OwnedBilinear` are real production paths (no more
  token returns).
- The cascade is end-to-end working on the user's RX 580: select
  EASU → dispatch real WGSL EASU → write to target. The spec's
  Recommended-tier milestone (60 FPS @ 1440p with the cascade)
  is reachable.
- `UpscalePass::record()` follows the same `record()` discipline
  as every other Phase 5.5 Track-A pass per ADR-075.
- The in-tree EASU shader closes ADR-076 step 2's outstanding
  follow-up.

### Negative

- `fsr_easu.wgsl` mirrors a vendor's published algorithm. ADR-076
  §Negative anticipates this risk; the WGSL port is committed under
  the algorithm's MIT release hash. If AMD reissues under a more
  restrictive license, the in-tree mirror moves behind the `fsr`
  cargo feature (already-documented escape valve).
- The runtime fallback path (`OwnedBilinear` on vendor failure)
  adds a second dispatch on the failure path — first-frame cost
  on error. Acceptable: error is rare; per-frame stats exclude
  frame 0 already.

### Neutral

- `PassContext::upscaler` is `Option<>`; passes that don't need it
  pay zero cost.

## Implementation

PR 1a of Phase 6 (per ADR-084):

1. `crates/engine-render/src/render_graph.rs` — `PassContext::upscaler`
   field.
2. `crates/engine-render/src/passes.rs` — `UpscalePass::record()`
   body.
3. `crates/engine-render/shaders/fsr_easu.wgsl` — new EASU + RCAS.
4. `crates/engine-render/shaders/bilinear_upscale.wgsl` — new
   GPU bilinear.
5. `crates/engine-render/src/upscale.rs` — `VendorFsr::upscale()`
   + `OwnedBilinear::upscale()` real dispatches.
6. `crates/engine-render/tests/upscale_dispatch.rs` — new test.

## References

### Algorithm provenance

- Lottes, T. *FidelityFX FSR 1.0 Algorithm*. GPUOpen, 2021.
  <https://gpuopen.com/fidelityfx-fsr-1-0/>.
- FSR 1.1 source release (MIT) — the WGSL port's reference:
  <https://github.com/GPUOpen-Effects/FidelityFX-FSR>.

### Prior engine ADRs

- [ADR-005](005-upscaler-provider-trait.md) — trait surface.
- [ADR-039](039-render-graph-abstraction.md) — the `Pass` trait
  the wired `record()` implements.
- [ADR-049](049-engine-gpu-wgpu-wrapper.md) — the `engine-gpu`
  surface the dispatch goes through.
- [ADR-063](063-shader-artefact-to-pipeline.md) — the shader
  artefact pipeline both new WGSL shaders flow through.
- [ADR-066](066-upscaler-vendor-cascade.md) — the cascade-select
  this ADR's `record()` body calls.
- [ADR-067](067-owned-onnx-temporal-upscaler.md) — the
  "degrade-but-keep-frames" property the fallback path realises.
- [ADR-068](068-phase-6-pr-slicing.md) — the PR 7 / PR 8 addendum
  that documented the current no-op.
- [ADR-075](075-track-a-pass-record-discipline.md) — the Phase
  5.5 record() discipline this ADR extends to the upscale pass.
- [ADR-076](076-fsr-2-spatial-fallback.md) — step 2 (the in-tree
  EASU shader) is realised by this ADR.
- [ADR-084](084-phase-6-pr-slicing.md) — Phase 6 PR slicing.
