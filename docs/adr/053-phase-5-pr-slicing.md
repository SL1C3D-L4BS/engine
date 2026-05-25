# ADR-053 — Phase 5 PR slicing (6-PR plan)

- Status: Accepted (planning decision; first PR lands when the audit
  remediation closes)
- Date: 2026-05-24
- Phase: 5 — RENDERING FOUNDATION (Track A)
- Companion: ADR-039 (render graph), ADR-040 (CSM), ADR-041 (IBL),
  ADR-042 (TAA), ADR-043 (cluster lights), ADR-044 (bindless heap),
  ADR-045 (texture compression), ADR-046 (oracle regression), ADR-047
  (frame-pacing CI gate), ADR-049 (engine-gpu wrapper)

## Context

Phase 4 was sliced into 4 PRs (sli front-end, VM/GC/verifier, hot-
reload/debugger/REPL, Slang toolchain + reproducibility). Each PR
landed one ADR or a tight pair and stood alone in CI. The result was
clean review history and a clean rollback story per increment.

Phase 5 has more surface than Phase 4: a new crate (`engine-gpu`,
ADR-049), a software rasterizer with its own oracle (ADR-046), eight
named subsystems (CSM, IBL, TAA, cluster lights, SSAO, bloom,
tonemap, upscale), and a milestone that's externally observable (RX
580 @ 60 FPS @ 1440p). The planning session chose to keep the Phase-4
PR cadence — six PRs, each landable independently, each with at
least one ADR realised or one milestone gate activated.

This ADR records the slicing so the future Phase-5 planning session
inherits it without re-debate. The contract is: each PR lands the
named ADRs' implementations and ships green CI on its own.

## Decision

### PR 1 — Rasterizer oracle + render-graph trait

Lands:
- The software rasterizer in `testbed/engine-raster/` (closes the
  stub). Pure CPU Rust, std-only, `std::simd` inner loop, tile-
  parallel via `engine-platform::JobGraph` (ADR-032). Phong shading
  exactly per spec Part IX.
- The `engine_render::render_graph` trait surface (ADR-039) —
  `Pass`, `Resource`, `RenderGraph`, `OracleAlternative` types.
  No GPU code yet; the trait compiles against a `MockDevice`.
- The oracle harness (ADR-046) — fixture pak format, comparison
  metric, exception parser, CLI surface (`engine raster`).
- Pixel-parity oracle's first 3 fixtures: sphere-on-plane, Cornell
  box, sponza-lite. All render through the CPU path twice (sanity
  check), one fixture in `--run-oracle` mode for CI.

Closes ADRs: 039 (implementation), 046 (implementation).

CI additions: oracle harness in CI as informational (continue-on-
error); becomes required when PR 6 lands.

Estimated size: medium (~3 500 LOC).

### PR 2 — `engine-gpu` + swapchain + bindless heap

Lands:
- New crate `crates/engine-gpu/` per ADR-049. `wgpu` as workspace
  dep. The `wgpu::` grep guard (ADR-049) added to CI.
- `Device`, `Swapchain`, `Buffer`, `Texture`, `Sampler`,
  `CommandEncoder`, `PipelineState` wrappers.
- `BindlessHeap` per ADR-044 (SRV + sampler heaps, free-list,
  generation tags, telemetry).
- Texture-compression import path (ADR-045) — at least BC7 + BC5
  + BC4 import via an owned-discipline subprocess wrapper around
  the chosen compressor (`intel_tex` or equivalent, ADR-045 §4).
- The asset pak format gains the `TextureMeta` record.

The renderer still renders nothing visible — the `render_graph`
from PR 1 now has a real GPU backend, but no passes are registered.
First `engine raster --backend gpu` invocations produce a cleared
swapchain.

Closes ADRs: 049 (implementation), 044 (implementation), 045
(implementation).

CI additions: `wgpu::` grep guard goes live (gate job).

Estimated size: large (~6 000 LOC).

### PR 3 — Deferred G-buffer + cluster lights + CSM

Lands:
- The geometry pass (`draw.opaque` per spec §IV.4.A): MRT
  G-buffer (albedo+roughness, normals+metallic, motion+depth)
  rendered into bindless-indexed texture slots.
- The cull pass (`cull`) — frustum culling on the compute path,
  producing an indirect draw buffer for the geometry pass.
- The cluster-light pass (`light.cluster`) per ADR-043 — 16×9×24
  grid, compute shader assignment, owned light SSBO.
- The CSM pass (`shadow`) per ADR-040 — 4 cascades, 4096² D32F
  atlas, practical-split with λ=0.6, 5×5 Vogel-disk PCF.
- The lighting accumulation pass (`draw.opaque.2`) — Cook-Torrance
  BRDF with cluster lights + CSM (no IBL yet, no GI yet; both come
  in PR 4).
- First end-to-end visible image. Three fixture additions to the
  rasterizer oracle (ADR-046): cluster-lights, shadow-heavy,
  combined-deferred. PR 1's oracle harness validates the CPU path
  matches the GPU path within ADR-046 thresholds.

Closes ADRs: 040 (implementation), 043 (implementation).

Estimated size: large (~7 500 LOC).

### PR 4 — IBL + post-FX (SSAO, bloom, tonemap, TAA)

Lands:
- IBL pass per ADR-041 (`probe.gi`): SH-L2 probe sampling baked at
  the CPU bake-stub stage, runtime trilinear interpolation. The
  CPU bake stub lands here so the test harness has reference data;
  the editor-driven bake UI is Phase 10.
- The full lighting accumulation now includes IBL diffuse + IBL
  specular (Karis split-sum).
- SSAO: HBAO-class screen-space AO via a small compute pass.
- Bloom: 5-mip downsample/upsample chain with energy-preserving
  filter.
- Tonemap: ACES Filmic (Stephen Hill fit).
- TAA per ADR-042: Halton (2,3) period-8 jitter, neighbourhood-clip
  rejection in YCgCo, motion-vector reprojection, disocclusion
  mask, velocity-aware sharpening. Cascade-jitter cross-check
  invariant with CSM.
- New oracle fixtures: IBL fixture, TAA-motion fixture, post-FX
  fixture.

Closes ADRs: 041 (implementation), 042 (implementation).

Estimated size: large (~6 500 LOC).

### PR 5 — UpscalerProvider trait + RX-580 milestone bench

Lands:
- The `UpscalerProvider` trait surface per spec §IV.4.A line 406
  (and the future ADR-005 expansion). Four implementations:
  - `Vendor::Dlss` — stub interface (real binding lands Phase 6
    when DLSS Streamline integration is in scope).
  - `Vendor::Fsr` — stub interface (real binding Phase 6).
  - `Vendor::Xess` — stub interface (real binding Phase 6).
  - `Owned::Bilinear` — placeholder owned fallback (the full
    ONNX-temporal owned upscaler is Phase 6 per spec line 1634;
    Phase 5 ships bilinear so the trait surface is end-to-end
    testable).
- The selection logic per ADR-005 (vendor > best match > owned).
- The "RX 580 milestone bench" binary —
  `bin/engine-bench-frame-pacing/` (the same binary ADR-047 calls
  for). Runs the Phase-5 standard scenario at 1440p with the RX
  580 quality preset; produces a JSON report.
- Bench runs informationally on PR 5 (not yet gating the build).
- First on-RX-580 measurement recorded in
  `docs/observatory/phase-5-milestone-baseline.md`.

Closes ADRs: trait surface for ADR-005 (spec realisation; the ADR
itself was a stub and an ADR-005-expansion is part of the Phase-0
sweep, not Phase 5).

Estimated size: medium (~3 000 LOC).

### PR 6 — Frame Pacing CI gate (ADR-047) + Phase 5 closure

Lands:
- The frame-pacing CI gate per ADR-047. The
  `bin/engine-bench-frame-pacing/` runs as a CI job on the self-
  hosted GPU runner; the gate becomes a required status check.
- Final RX-580 milestone validation: the standard scenario hits
  p99 ≤ 18.3 ms and σ ≤ 1.04 ms on the runner.
- `engine.toml` `phase` updates to `"5"`. README's Status section
  gains a Phase-5 paragraph.
- Engine Core v0.2 tag (the renderer is the second major
  milestone after Engine Core v0.1).
- Any final oracle-exception entries needed for known GPU driver
  divergences are recorded in `docs/audit/oracle-exceptions.md`.

Closes ADRs: 047 (CI gate activation), spec §IV.5 realisation.

Estimated size: small (~1 500 LOC; mostly CI workflow + the bench
binary's polish + manifest updates).

## Consequences

- **Six PRs, ~28 000 LOC estimate end-to-end.** Phase 4 was ~12 000
  LOC across 4 PRs; Phase 5 is over twice the surface as expected
  for the first GPU-touching phase.
- **Each PR ends in green CI on its own.** Per-PR rollback is real;
  if PR 3's cluster pass turns out to be wrong-architecture, PR 4
  isn't blocked from rebasing on a different cluster strategy.
- **Oracle fixtures grow incrementally.** PR 1 ships 3; PR 3 adds 3
  more; PR 4 adds 3 more; PR 5 adds 1; PR 6 closes the set. Each
  PR's oracle additions are reviewable in isolation.
- **The milestone is measurable from PR 5 onward.** Pre-PR-5 there's
  no full pipeline to measure; PR 6 is the formal gate.
- **No PR depends on a Phase-5 ADR not yet landed.** All Phase-5
  ADRs (039–048, 049, 053) land as part of the audit-remediation
  packets *before* PR 1 of Phase 5 begins. PR sequencing is
  implementation only.

## Risks and tradeoffs

- **PR 3 is the riskiest** — it stitches geometry, culling, clustering,
  and shadow into one PR. Smaller alternatives split shadows out,
  but the deferred lighting equation reads cluster + CSM together,
  and splitting forces a half-broken intermediate. Decided: bundled.
- **The bench-binary's owned discipline** (own arg parser, own JSON
  emitter — same as the sampling-profiler CLI) means PR 5 includes
  the harness, not just the trait. Acceptable.
- **PR 6 depends on the self-hosted GPU runner being operational.**
  Runbook + cold-spare per ADR-047 §Risks; the Phase-5 schedule must
  budget for runner provisioning before PR 6 ships.
- **If a PR slips a target ADR**, the next PR may need to absorb
  the work. Documented: the slice is a default; deviations require
  a comment on this ADR (no new ADR).

## Alternatives considered

- **3 large PRs** (oracle+graph+GPU surface / full deferred pipeline
  / post-FX + upscale + milestone). Bigger atomic increments; harder
  to review; longer time between green-CI hits. Rejected.
- **10+ small PRs** (one feature per PR). Maximum granularity; CI
  cost dominates; review fatigue real on a 10-PR series. Rejected.
- **Different ordering** — e.g. ship the GPU surface (PR 2) before
  the rasterizer + graph (PR 1). Rejected because the oracle is
  the verification mechanism for the GPU path; building the GPU
  path with no way to oracle-test it is the spec-R-02-violating
  pattern.
- **Bundle ADR-049's engine-gpu crate creation into PR 1.** Tempting
  but PR 1 already ships the rasterizer + graph + oracle harness;
  adding a new crate doubles the PR size. Kept separate.

## Verification

- This ADR closes when Phase 5 PR 6 lands. Until then, it is the
  *plan*; PRs that arrive should match the slice or explain in
  their description why they deviate.
- The Phase-5 planning session (the one after the audit closes) reads
  this ADR as the starting point; it does not re-debate the slice.
  It produces the per-PR implementation tickets, the per-PR review
  assignment, and the schedule.
- No code changes from this ADR alone — pure planning record.
