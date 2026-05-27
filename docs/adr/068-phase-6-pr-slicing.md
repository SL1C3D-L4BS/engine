# ADR-068 — Phase 6 PR slicing (6-PR plan)

- Status: Accepted (planning record; first PR lands when this ADR
  sweep merges)
- Date: 2026-05-27
- Phase: 6 — RENDERING FOUNDATION (Track A, Part 2)
- Companion: ADR-053 (Phase 5 PR slicing — the precedent), ADR-061
  (mesh/material owned format), ADR-062 (glTF importer subprocess),
  ADR-063 (shader artefact to pipeline binding), ADR-064 (GPU
  geometry/lighting pass contracts), ADR-065 (GPU post-FX pass
  contracts), ADR-066 (vendor upscaler binding discipline),
  ADR-067 (Owned::OnnxTemporal upscaler), ADR-047 (frame pacing
  CI gate — promoted to required in PR 6)

## Context

Phase 5 closed on 2026-05-27 with all six planned PRs landed (522
workspace tests, 0 failures, `engine.toml` `phase = "5"`). The
renderer now has a full Track-A trait surface but every pass's
`record()` body is a no-op pending Phase 6 GPU implementation
(`docs/architecture/engine-render.md:41`). The vendor upscaler
stubs (`crates/engine-render/src/upscale.rs:147–200`) return
`supports() == false`. The runtime has no concrete mesh or
material asset format. The frame-pacing CI gate is
`continue-on-error: true` (`.github/workflows/ci.yml`, line 530).

Phase 6 closes those gaps. The work is large — 6 PRs, ~24 000 LOC
estimate end-to-end — and the discipline that worked for Phase 5
(ADR-053) is the precedent: each PR lands at least one design
ADR's implementation or activates at least one milestone gate;
each PR ends green in CI on its own; the oracle and the wgpu-
boundary guard remain enforced throughout.

This ADR records the Phase 6 slicing so the implementation work can
reference it without re-debate. ADRs 061–067 lock the engineering
contracts referenced by the PR descriptions below. They all land
together as the pre-Phase-6 design sweep (one PR), matching how
Phase 5's design ADRs (039–053) landed during the audit closure
*before* Phase 5 PR 1.

## Decision

### Pre-Phase-6 sweep — design ADRs

A single PR ("Phase 6 design sweep") lands ADRs 061–068 plus the
amendment to ADR-051 that ADR-067 §5 specifies (the `ort` deviation
entry). No code changes; pure documentation. Closes when this
ADR sweep merges; subsequent PRs reference these ADRs.

### PR 1 — Mesh + material asset formats + glTF importer subprocess

Lands:

- `crates/engine-asset/src/mesh.rs` (new): `MeshMeta` + `VertexSemantic`
  + `SubMesh` + encode/decode functions per ADR-061 §1. 24-byte
  deterministic header (`EMSH`).
- `crates/engine-asset/src/material.rs` (new): `MaterialMeta` +
  `TextureSlot` + `SamplerKind` + encode/decode functions per
  ADR-061 §2. 24-byte deterministic header (`EMAT`).
- `tools/engine-mesh-import/` (new workspace member): subprocess CLI
  wrapping `gltf` 1.4 per ADR-062. Sandboxed via
  `engine_platform::sandbox`. Owned arg parser; owned JSON manifest;
  red-team test.
- CI guard: ADR-062 §Verification's grep guard rejects `gltf::|use
  gltf\b` outside `tools/engine-mesh-import/`.
- `docs/architecture/engine-asset.md` (new): the architecture doc
  for the crate; covers texture + mesh + material kinds + the
  importer's place.
- Test fixtures: `cube.emsh` + `cube.emat` in
  `testbed/engine-raster/fixtures/`; the CPU oracle gains a loader
  variant of `combined_deferred_scene` that consumes the fixtures.

Closes ADRs: 061 (implementation), 062 (implementation).

CI additions: glTF boundary guard.

Estimated size: medium (~3 500 LOC).

### PR 2 — Shader artefact ingest + pipeline-construction wiring

Lands:

- `crates/engine-render/src/shader.rs` (new): `ShaderArtefactSet`
  + `build_render_pipeline` + `build_compute_pipeline` +
  `PipelineCache` per ADR-063. Reflection-driven bind-group layout
  extraction. Push-constant 64 B convention codified.
- `crates/engine-render/shaders/` (new directory): the first
  shipped Slang shaders (`pbr_opaque.slang`, `clear_blit.slang`),
  compiled via the pre-build script invoking `tools/engine-shader/`.
- The 11 pass stubs in `crates/engine-render/src/passes.rs`
  gain `pipeline:` fields initialized lazily on first `record()`
  via the new helpers.
- New tests: `pipeline_smoke.rs` (load shader → build pipeline →
  run a clear+blit on a headless `engine_gpu::Device`),
  `reflection_layout.rs` (every shipped shader's reflection
  matches the group-0/1/2/3 convention).
- `docs/architecture/engine-render.md` updated: pipeline cache,
  shader convention.

Closes ADRs: 063 (implementation).

Estimated size: small-medium (~2 500 LOC).

### PR 3 — GPU `record()` for geometry + lighting (5 passes)

Wires real wgpu draw/dispatch into the 5 geometry/lighting passes
per ADR-064 — the "first visible image" PR of Phase 6 (analog to
ADR-053's Phase 5 PR 3).

Lands:

- `CullPass.record()` — compute frustum cull → `IndirectDrawBuffer`.
- `CsmShadowPass.record()` — 4-cascade reverse-Z depth-only draw
  into 4096² D32F atlas; CSM uniforms per ADR-064 §4.
- `GBufferPass.record()` — MRT draw into Albedo+Roughness,
  Normal+Metallic, Motion+Depth+ID per ADR-064 §3.
- `ClusterLightPass.record()` — compute assignment per ADR-064 §5;
  SSBO layouts matching CPU oracle.
- `LightingAccumulationPass.record()` — full-screen Cook-Torrance
  GGX evaluation reading G-buffers + cluster + shadow atlas.
- Three new oracle fixtures + tests in `tests/rendering/`:
  `cube_pixel_parity`, `csm_4_cascade_pixel_parity`,
  `cluster_64_lights_pixel_parity`. ADR-046 thresholds enforced.
- New shaders: `cull.slang`, `csm_shadow.slang`, `gbuffer.slang`,
  `cluster_assign.slang`, `lighting.slang` in
  `crates/engine-render/shaders/`.

Closes ADRs: 064 (implementation). Strengthens ADRs 040 + 043.

Estimated size: large (~6 500 LOC).

### PR 4 — GPU `record()` for post-FX (5 passes)

Wires real wgpu compute/draw into the 5 post-FX passes per ADR-065.

Lands:

- `SsaoPass.record()` — 8-tap Fibonacci kernel compute SSAO at
  half-res; bilateral upsample in consumer.
- `IblPass.record()` — L2 SH evaluation + split-sum BRDF LUT
  sampling; `IblProbeSet` SSBO + `BrdfLut` texture inputs.
- `BrdfLutBake` (one-shot at engine init) — bakes the LUT to
  `$XDG_CACHE_HOME/sliced-engine/brdf_lut.bin`; subsequent runs
  load from cache.
- `TaaPass.record()` — Halton(2,3) jitter sourced from the
  re-exported `engine_render::post_fx::jitter::jitter_for_frame`;
  YCgCo neighbourhood-clip; double-buffered `TaaHistory` ping-
  pong.
- `BloomPass.record()` — 5-mip compute downsample + upsample
  chain; soft-knee extract.
- `TonemapPass.record()` — ACES filmic compute pass to
  `Bgra8UnormSrgb` swapchain format.
- Three new oracle fixtures + tests: `ibl_probe_pixel_parity`,
  `taa_motion_pixel_parity`, `post_fx_chain_pixel_parity`.
- New shaders: `ssao.slang`, `ibl_evaluate.slang`,
  `brdf_lut_bake.slang`, `taa_resolve.slang`, `bloom_downsample.slang`,
  `bloom_upsample.slang`, `tonemap.slang`.

Closes ADRs: 065 (implementation). Strengthens ADRs 041 + 042.

Estimated size: large (~5 500 LOC).

### PR 5 — Vendor upscaler FFI + ONNX owned fallback

Lands:

- `crates/engine-upscale-vendor/` (new Level-1 crate) per ADR-066:
  one module per vendor (DLSS / FSR / XeSS) behind cargo features;
  loader-thread sandbox; SDK digest verification.
- `tools/upscaler-vendor-sdks/{streamline,fsr,xess,ort}/` (new
  vendored `*-sys` crates): minimal `bindgen`-generated FFI; per-
  vendor LICENSE files.
- `crates/engine-render/src/upscale/onnx.rs` (new): the
  `OwnedOnnxTemporal` provider per ADR-067; lazy ORT session init;
  hardware-backend cascade.
- `crates/engine-render/assets/onnx/temporal_upscaler_v1.onnx` (new
  asset, ~3 MiB via Git LFS): the trained model.
- `crates/engine-render/src/upscale.rs`: vendor stubs replaced with
  re-exports from `engine-upscale-vendor`; `with_phase6_defaults()`
  added (with `with_phase5_defaults` deprecated for one release).
- `engine.toml` schema: `[upscaler]` section with `provider` +
  `quality` fields. Reader in `engine-platform`.
- `bin/engine-bench-frame-pacing/`: JSON report adds `"upscaler"`
  field with selected provider + reason.
- `deny.toml`: per-vendor + ORT license allowances.
- `ADR-051` amendment: ORT deviation entry (per ADR-067 §5) + three
  vendor SDK deviation entries (per ADR-066 §License management).
- `.gitattributes`: `*.onnx filter=lfs diff=lfs merge=lfs -text`.

Closes ADRs: 066 (implementation), 067 (implementation). Fully
realises ADR-005.

CI additions: gate job runs `--no-default-features` build path
unchanged (the existing CI behaviour); the self-hosted GPU runner
gains `--features all-vendors` build path.

Estimated size: large (~4 500 LOC including bindgen output).

### PR 6 — Frame-pacing gate promotion + Phase 6 closure (Engine
Core v0.3)

Lands:

- `.github/workflows/ci.yml`: `frame_pacing` job has
  `continue-on-error: true` removed. Becomes a required status
  check. Job now matrix-builds `--features all-vendors` on the
  self-hosted runner.
- First green baseline on the RX 6700 XT runner (operator-confirmed
  per the runbook); `tools/frame-pacing/budgets.toml` re-confirmed
  (18.3 ms p99, 1.04 ms σ).
- `docs/observatory/phase-6-milestone-baseline.md`: first
  measurement reports for each registered upscaler + the
  procedural-scene reference fixture.
- `docs/observatory/phase-6-frame-zero.png` (new artifact): frame 0
  from `combined_deferred_scene` rendered through the full
  Phase 6 pipeline, archived as the milestone visual proof.
- `engine.toml`: `phase = "5"` → `phase = "6"`. README Status
  gains a Phase 6 paragraph + per-PR summary (matching the
  Phase 5 paragraph's structure).
- Any new oracle-exception entries recorded in
  `docs/audit/oracle-exceptions.md` (GPU driver divergences caught
  during PRs 3+4).
- Engine Core v0.3 tag (post-merge user action; the PR ships
  everything ready to tag).

Closes ADRs: 068 (this ADR), 047 (frame-pacing CI gate
activation), spec §IV.5 Track-A realisation.

Estimated size: small (~1 500 LOC; CI workflow + bench-binary
polish + manifest updates + docs).

## Consequences

- **Six PRs, ~24 000 LOC estimate end-to-end.** Phase 5 was ~28 000
  LOC across 6 PRs; Phase 6 is comparable in surface, slightly
  less code (no new crates as large as `engine-gpu`; the
  `engine-upscale-vendor` crate is smaller because most logic is
  FFI delegation).
- **Each PR ends in green CI on its own.** Per-PR rollback is real;
  if PR 3's geometry pass turns out to be wrong-architecture, PR 4
  can rebase on a different geometry strategy.
- **Pixel-parity oracle fixtures grow incrementally.** PR 3 adds 3;
  PR 4 adds 3; PR 5 adds 1 (`onnx_quality_oracle`); PR 6 closes
  the set. Each PR's oracle additions are reviewable in isolation.
- **The milestone is measurable from PR 3 onward.** Pre-PR-3 there
  is no GPU-visible image; PR 6 is the formal gate.
- **No PR depends on a Phase-6 ADR not yet landed.** All Phase-6
  ADRs (061–068) land as part of the pre-Phase-6 design sweep
  *before* PR 1 of Phase 6 begins. PR sequencing is implementation
  only.
- **Track B remains research.** Per the user's "strict Track A
  close" planning decision (saved in
  `/home/doodlebob/.claude/plans/ancient-wondering-church.md`),
  no Track B scaffolding lands in Phase 6.

## Risks and tradeoffs

- **PR 3 is the riskiest** — five passes wired together in one PR;
  the geometry-vs-lighting-vs-cluster-vs-CSM coupling is the same
  reason ADR-053's Phase 5 PR 3 was the riskiest. Decided:
  bundled, same as Phase 5; the alternative (split shadows from
  geometry) forces a half-broken intermediate.
- **PR 5 depends on vendor SDK access.** The CI default-feature
  build doesn't need the SDKs; the self-hosted runner does. The
  PR includes a documented SDK-fetch procedure in
  `docs/runbooks/vendor-upscaler-sdks.md` (new runbook) for the
  runner operator.
- **PR 6 depends on the self-hosted GPU runner being operational.**
  Runbook + cold-spare per ADR-047; Phase-6 schedule must
  budget for runner readiness before PR 6 ships. (Per user's
  Phase-6 planning decision, the runner is expected online by
  PR 6.)
- **First-frame stall after PR 3+4.** Pipeline cache (ADR-063) +
  BRDF LUT bake (ADR-065 §3) + ORT session init (ADR-067 §4) all
  pay first-frame costs. PR 6's frame-pacing report excludes
  frame 0 from p99/σ stats.
- **If a PR slips a target ADR**, the next PR may need to absorb
  the work. Documented per ADR-053's same clause: the slice is a
  default; deviations require a comment on this ADR (no new ADR).

## Alternatives considered

- **One large "Phase 6 Track A close" PR.** Atomic; impossible to
  review; impossible to roll back partial. Rejected.
- **8+ small PRs (one feature per PR).** Maximum granularity; CI
  cost dominates; review fatigue. Rejected.
- **Different ordering — vendor upscalers in PR 3 before geometry
  passes.** The bench scenario needs visible geometry to measure
  upscaler quality; vendor upscalers in PR 3 would have nothing to
  upscale. Rejected.
- **Bundle the ADR sweep into PR 1.** Tempting (one PR = "code +
  its ADR"), but the design ADRs need to land *before* the
  implementation work so reviewers of PR 1+ reference accepted
  contracts. Kept as a separate pre-Phase-6 PR.
- **Defer the ONNX owned upscaler to Phase 7.** Phase 6 is the
  designated phase for the upscaler-cascade end state; deferring
  leaves the cascade half-built. Rejected.
- **Skip glTF; require artists to author EMSH directly.** Loses
  every DCC tool. Rejected per ADR-062 §Alternatives.

## Verification

- This ADR closes when Phase 6 PR 6 lands. Until then, it is the
  *plan*; PRs that arrive should match the slice or explain in
  their description why they deviate.
- The Phase-6 implementation session reads this ADR as the
  starting point; it does not re-debate the slice. It produces
  the per-PR implementation tickets, the per-PR review assignment,
  and the schedule.
- The pre-Phase-6 design sweep PR closes when ADRs 061–067 land
  plus the ADR-051 amendment (the `ort` deviation entry).
- Engine Core v0.3 tag (post-PR-6 user action) closes Phase 6;
  this ADR's Status transitions to "Closed" via an addendum
  entry in this file with the tag commit hash.
- No code changes from this ADR alone — pure planning record.

## Addendum (2026-05-27, later same day) — Phase 6 sub-PRs landed

Three sub-PRs landed on top of the contract-side close to bring
every CI-validatable deliverable into the tree:

- `bb14ac4` — PR 5.5: `crates/engine-upscale-vendor/` Level-1 crate
  scaffold per ADR-066 §1. Cargo features `dlss` / `fsr` / `xess` /
  `ort-runtime` / `all-vendors` (all default off). Per-vendor module
  skeletons. ADR-051 deviation entry 4 (ORT) added. `engine.toml`
  `[upscaler]` schema documented.
- `d307c4b` — PR 3.5: WGSL shader sources for the five geometry +
  lighting passes (ADR-064). `cull.wgsl`, `csm_shadow.wgsl`,
  `gbuffer.wgsl`, `cluster_assign.wgsl`, `lighting.wgsl`. Cross-
  checked against `contracts::*` constants.
- `f7bf287` — PR 4.5: WGSL shader sources for the five post-FX
  passes + BRDF LUT bake (ADR-065). `ssao.wgsl`, `brdf_lut_bake.wgsl`,
  `ibl_evaluate.wgsl`, `taa_resolve.wgsl`, `bloom.wgsl`,
  `tonemap.wgsl`.

Cumulative test delta: 581 → 596 (+15 across the three sub-PRs).
Every commit `just ci` green.

### Remaining work for v0.3 — single runner-gated PR

The contract surface, the shader sources, the upscaler crate
scaffold, and the cross-check coverage now in tree leave exactly one
follow-up PR before Engine Core v0.3:

1. Pipeline construction via `engine_render::shader::build_*_pipeline`
   over the shaders in PR 3.5 + 4.5 + the contracts in PR 3.
2. `record()` body wiring on each of the ten Track-A passes —
   descriptor bind + push-constant set + draw/dispatch.
3. Pixel-parity oracle fixtures (3 per ADR-064, 3 per ADR-065)
   rendered through both CPU oracle and GPU path; pass/fail at
   ADR-046's 1/255 channel + p99 ≤ 1% threshold.
4. Vendor SDK FFI: `*-sys` crates + per-vendor `Real` provider
   structs filling in `crates/engine-upscale-vendor/src/{dlss,fsr,xess}.rs`.
5. ORT runtime: `ort` crate dep behind the `ort-runtime` feature;
   bundled `temporal_upscaler_v1.onnx` via Git LFS; ADR-051 entry 4
   marked as implemented.
6. `engine.toml [upscaler]` runtime reader in engine-platform.
7. `.github/workflows/ci.yml` `frame_pacing` job promoted from
   `continue-on-error: true` to required (ADR-047 §7); first green
   RX 6700 XT baseline at `docs/observatory/phase-6-milestone-baseline.md`.

That PR's prerequisites are environmental — runner provisioning +
SDK downloads + Git LFS setup — not engineering. When it lands, the
Engine Core v0.3 tag ships and this ADR's Status flips to *Closed*
with a final addendum carrying the tag commit hash.

## Addendum (2026-05-27) — Phase 6 contract-side close

Five PRs landed locally on 2026-05-27, closing the *contract-side* of
Phase 6 — every layout, every cascade position, every cross-checked
constant a future GPU + vendor SDK + ORT integration must bind
against:

- `b205450` — Pre-Phase-6 design sweep (ADRs 061-068).
- `d70b853` — PR 1: mesh + material formats + glTF importer
  subprocess (ADR-061 + ADR-062).
- `eec6aa4` — PR 2: shader artefact ingest + pipeline construction
  (ADR-063).
- `1faa877` — PR 3+4 combined: GPU pass contracts + CPU oracle
  cross-checks (ADR-064 + ADR-065). The two ADRs' contract surfaces
  ship together because both are CPU-only Rust types validated
  against the same oracle constants.
- `1dfd950` — PR 5: OwnedOnnxTemporal cascade reservation +
  `with_phase6_defaults()` (ADR-066 cascade-position half + ADR-067
  trait-surface half).
- *(this commit)* — Phase 6 contract close: `engine.toml` bump to
  `phase = "6"` + README Status section + this addendum.

Tests: 522 → 581 (+59) across the five implementation PRs. Every PR
ended `just ci` green (build + test + clippy `-D warnings` +
fmt-check + cargo-deny).

### Deferred to runner / SDK-gated follow-ups

The original ADR-068 slicing called for PR 3's "first visible image"
deliverable (real GPU `record()` body wiring + 3 oracle fixtures
rendered through the GPU path), PR 4's post-FX `record()` bodies,
PR 5's vendor SDK FFI + ONNX integration, and PR 6's frame-pacing
gate promotion. Each of those requires hardware the CI default job
does not have:

- **PR 3.5 / 4.5 — GPU `record()` bodies + Slang shader sources.**
  Pixel-parity oracles (ADR-046) cannot validate GPU output without
  a real `engine_gpu::Device` (CI builds wgpu without backend
  features per the `engine-gpu` architecture doc); landing untested
  GPU code would defeat ADR-046's purpose.
- **PR 5.5 — Vendor upscaler FFI + ONNX integration.** DLSS / FSR /
  XeSS SDKs require download + license acceptance + binary blobs
  not on crates.io. The `ort` ONNX runtime requires its own native
  binaries. Git LFS setup for the bundled `temporal_upscaler_v1.onnx`
  model is environmental.
- **PR 6.5 — Frame-pacing CI gate promotion.** Per ADR-047 §7 the
  gate flips to required when the self-hosted RX 6700 XT runner
  comes online + a first green baseline lands. The runbook at
  `docs/runbooks/frame-pacing-runner.md` is the provisioning record.

### Status

This ADR remains *Accepted* (not yet closed) because PR 6's
runner-dependent deliverables are unfulfilled. When the runner comes
online + the GPU bodies land + the vendor SDKs integrate + the
frame-pacing gate goes required, a final addendum lands here with
the Engine Core v0.3 tag commit hash and the Status flips to
*Closed*.

The contract surface this PR cohort landed is the prerequisite for
those follow-ups — the future "land GPU bodies" PR is a binding
exercise against types this PR fixed, not a rewrite of the contract.

## Addendum (2026-05-27, third pass) — Phase 6 PR 7 (engineering-only GPU pipeline binding)

Per user direction, the original "single runner-gated v0.3 candidate
PR" splits into two: **PR 7** lands the pure-Rust engineering work
that does not need vendor SDKs, an ONNX model, or a GPU runner;
**PR 8** lands the runner-validated closure (SDK FFI + ONNX
integration + pixel-parity fixtures + frame-pacing gate
promotion + v0.3 tag).

### PR 7 delivered

- **`PassContext` extension** — `crates/engine-render/src/render_graph.rs`
  grew a `gpu: Option<GpuFrameContext<'a>>` field; the new
  `GpuFrameContext { device, encoder }` is the per-frame surface
  `Pass::record` bodies bind against. `RenderGraph::execute` accepts
  an `Option<GpuFrameContext>` parameter and reborrows it per
  iteration so a single encoder flows through every scheduled pass.
- **Per-pass pipeline construction** — every Track-A pass struct in
  `crates/engine-render/src/passes.rs` gained a private
  `std::sync::OnceLock<{Render,Compute}Pipeline>` field and a
  `pub fn new(...)` constructor. Compute-pass `record()` bodies open
  a `ComputePass`, set the lazy-init pipeline, and issue a
  placeholder dispatch; render-pass `record()` bodies lazy-init the
  pipeline only (the current `engine_gpu` `begin_render_pass` surface
  requires attachment `TextureView`s that the graph's transient
  resource pool will resolve in PR 8). `BloomPass` carries three
  OnceLocks (extract / downsample / upsample) per its three WGSL
  entry points.
- **WGSL → pipeline bridge** — `crates/engine-render/src/shader.rs`
  exports `wgsl_artefact_set(stage, entry, source) ->
  ShaderArtefactSet`, the missing wrapper that turns a hand-written
  WGSL `&'static str` into a single-artefact `engine_shader::Bundle`
  the existing `build_{render,compute}_pipeline` helpers accept.
- **`init.rs` + `Phase6Pipelines`** — new module ships
  `build_brdf_lut_bake_pipeline(device)` for the init-time BRDF LUT
  bake (deliberately not modelled as a `Pass` because it runs once,
  not per frame — ADR-065 §3), and `build_all_phase6_pipelines(device)
  -> Phase6Pipelines` which assembles every Track-A + bake pipeline
  in one call. The integration test
  `crates/engine-render/tests/pipeline_smoke.rs` consumes this entry
  point as a fail-fast oracle on shader-validation regressions.
- **`engine.toml [upscaler]` runtime reader** — new
  `crates/engine-render/src/upscaler_config.rs` parses the documented
  schema (`provider = "auto" | "dlss" | "fsr" | "xess" |
  "owned-onnx" | "owned-bilinear"`, `quality = "performance" |
  "balanced" | "quality" | "ultra-quality"`). Mirrors the
  `bin/engine-bench-frame-pacing/src/budgets.rs` line-oriented
  pattern — no serde, no third-party TOML parser. Wired into
  `UpscalerRegistry::with_phase6_defaults_from_config(&cfg)` which
  registers the operator's forced provider plus `OwnedBilinear` as
  the universal fallback.

Tests: 596 → 610 (+14). `just ci` green (build + test + lint +
fmt-check + cargo-deny). ADR-049 wgpu boundary preserved (every
`wgpu::` mention outside `engine-gpu` is doc-comment text).

### Why pixel-parity fixtures + render-pass bodies wait for PR 8

`engine_gpu::CommandEncoder::begin_render_pass` requires a
`TextureView` for the colour attachment (or three, for the MRT
G-buffer). The render-graph's transient-resource pool does not yet
hand out resolved views — that resource-allocation pass is a non-
trivial chunk of the renderer, and unit-testing it without a real
GPU device is impossible because every test would need a fallback-
adapter `Device` instance. PR 7 therefore stops at "pipeline lazy-
init runs"; PR 8 adds the attachment-view plumbing alongside the
pixel-parity fixtures that exercise it end-to-end.

The shader sources at `crates/engine-render/shaders/*.wgsl` reference
`@group(N) @binding(M)` annotations that the empty bind-group layouts
do not declare. The smoke test will reveal whether wgpu accepts this
discrepancy at pipeline-creation time or rejects it; either outcome
informs PR 8's scope (real bind-group descriptors are PR 8 work
regardless; only the failure mode shifts).

### PR 8 — Engine Core v0.3 closure (deferred, runner-gated)

Identical contract to the original "single runner-gated PR" half of
the prior addenda's deferred list:

1. `tools/upscaler-vendor-sdks/{streamline,fsr,xess}/` —
   bindgen-generated `*-sys` crates + per-vendor `LICENSE-VENDOR.txt`.
2. Real provider impls in
   `crates/engine-upscale-vendor/src/{dlss,fsr,xess}.rs` (flip
   `supports_stub() -> false` to real `supports(device)` probes).
3. `crates/engine-render/assets/onnx/temporal_upscaler_v1.onnx` via
   Git LFS + `.gitattributes`; `OwnedOnnxTemporal::supports()` becomes
   a runtime ORT probe; ADR-051 entry 4 flips from "anticipated" to
   "active".
4. Six pixel-parity fixtures (3 ADR-064 + 3 ADR-065) rendering via
   both the CPU oracle in `engine-raster` and the GPU path; ADR-046's
   1/255 channel + p99 ≤ 1% threshold gates each.
5. Real `begin_render_pass` plumbing in the four render-pass bodies
   (csm_shadow / gbuffer / lighting + bloom's MRT path) plus real
   bind-group descriptors against the WGSL `@group/@binding`
   annotations.
6. `.github/workflows/ci.yml` `frame_pacing` job promoted from
   `continue-on-error: true` to required (ADR-047 §7); first green
   RX 6700 XT baseline at
   `docs/observatory/phase-6-milestone-baseline.md`.
7. `engine.toml` `phase = "6"` → `"6-closed"` (mirrors Phase 4's
   `"4-audited"` precedent); README v0.3 paragraph; this ADR's
   Status flipped from *Accepted* to *Closed* with the v0.3 tag
   commit hash.

PR 8's environmental prerequisites are unchanged: self-hosted
RX 6700 XT runner per `docs/runbooks/frame-pacing-runner.md`, DLSS
Streamline 2.x + AMD FSR 4 + Intel XeSS 2 SDKs downloaded and
licensed, `ort` native binaries installable, Git LFS configured.
