# ADR-084 — Phase 6 PR slicing (8-PR plan)

- Status: Accepted (planning decision; first PR lands when this ADR
  sweep merges)
- Date: 2026-05-28
- Phase: 6 — NEURAL RENDERING & GAUSSIAN SPLATTING (Engine Core v0.4)
- Companion: ADR-053 (Phase 5 precedent), ADR-068 (Phase 5.5 record
  — stays Closed; this ADR supersedes only its v0.4-milestone
  slicing role), ADR-069 (engine vs spec phase reconciliation),
  ADR-077–083 (Phase 6 design ADRs landing alongside this one)

## Context

Phase 5.5 closed 2026-05-28 with `engine.toml phase = "5.5-closed"`,
Engine Core v0.3 tag, 640 workspace tests passing. ADR-069
reconciled the engine's prior phase naming with the spec: what the
engine briefly called "Phase 6" was Phase 5 GPU-binding closure
(renamed Phase 5.5); the spec's Phase 6 (3DGS + neural rendering +
working vendor cascade) opens with a fresh track now.

The spec Phase 6 deliverable (lines 1633–1637):

> Portfolio: 3DGS renderer · Owned ONNX inference · Vendor
> upscaler integration. Milestone: 3DGS scene > 60 FPS. DLSS 4 /
> FSR 4 / XeSS 2 integrated, owned fallback. Books: Real-Time
> Rendering 4 / Deep Learning (Goodfellow).

Phase 5.5 left two load-bearing facts that PR 1a closes:

1. `UpscalePass::record()` is a no-op (per ADR-083's context). The
   cascade selects providers correctly but the dispatch is a no-op
   — the cascade's runtime hasn't been wired through `PassContext`
   yet. The Phase 5.5 PR 7 / PR 8 addendum on ADR-068 documented
   this.
2. Six oracle exceptions outstanding. ADR-081 sunsets 4 of them
   (engine-fix); converts `post_fx_chain` to permanent
   architectural-divergence; the `cube` exception stays vendor-driver.

Phase 6 sliced as **PR 0 (design sweep) + 7 implementation PRs**,
following ADR-053 + ADR-068 precedent: each PR lands at least one
ADR's implementation or activates one milestone gate; each PR ends
green `just ci`; the oracle + wgpu-boundary guards stay enforced
throughout.

## Decision

### Pre-Phase-6 sweep — PR 0 (design ADRs)

A single PR lands 8 design ADRs:

- ADR-077 — 3DGS architecture
- ADR-078 — ESPL asset format + glTF KHR_gaussian_splatting reader
- ADR-079 — Vendor SDK FFI discipline (DLSS Streamline + FSR 4 + XeSS 2)
- ADR-080 — ONNX v1 training pipeline
- ADR-081 — Oracle exception sunset + ADR-046 amendment
- ADR-082 — `engine-config` Level-1 crate
- ADR-083 — `UpscalePass::record()` wiring + in-tree EASU shader
- ADR-084 — This ADR (Phase 6 PR slicing)

ADR-067 also gains a *third amendment* tracked for landing in PR
4 (v1 trained model + ROCm explicit-disable + achieved-SSIM
clause).

ADR-051 gains four new deviation entries tracked across PR 3
(entries 5, 6, 7 — DLSS Streamline, FSR 4 SDK, XeSS 2 SDK) and
PR 4 (entry 4 flips from active token to active inference).

No code changes in PR 0. Pure documentation.

### PR 1a — Oracle closures + `UpscalePass::record()` wiring + EASU shader

Bundles the post-v0.3 follow-ups + the cascade-runtime wiring.

Engine fixes (close 4 oracle exceptions to strict 1/255):

- `crates/engine-render/shaders/lighting.wgsl` — CSM cascade
  projection (closes `csm_4_cascade`).
- `crates/engine-render/shaders/lighting.wgsl` — windowed
  inverse-square attenuation (closes `cluster_64_lights`).
- `crates/engine-render/tests/pixel_parity/ibl_probe.rs` — real
  BRDF LUT bind (closes `ibl_probe`).
- `taa_motion` — no code change; register-only sunset (inherits
  cube floor; documented per ADR-081 §1).

Permanent exception conversions (per ADR-081):

- `post_fx_chain` → architectural divergence (ADR-046 category 4).
- `cube` stays vendor-driver (pre-existing).

`UpscalePass::record()` wiring (per ADR-083):

- `PassContext::upscaler` field added.
- `UpscalePass::record()` body: real registry-select + dispatch.
- `OwnedBilinear::upscale()` real GPU dispatch (replacing CPU
  oracle delegation).
- `VendorFsr::upscale()` real EASU dispatch.

New shaders:

- `crates/engine-render/shaders/fsr_easu.wgsl` (per ADR-076 step 2 +
  ADR-083).
- `crates/engine-render/shaders/bilinear_upscale.wgsl` (per
  ADR-083).

Closes ADRs: 081 (oracle sunset + ADR-046 amendment), 083
(UpscalePass wiring + EASU shader).

### PR 1b — `engine-config` Level-1 crate refactor

Mechanical cleanup. Closes Phase 5.5 PR 7.5 finding #15.

- New `crates/engine-config/` with public `Config / Section / Value
  / parse()` surface.
- Three call sites become thin adapters:
  `crates/engine-render/src/upscaler_config.rs`,
  `bin/engine-bench-frame-pacing/src/budgets.rs`,
  `crates/engine-script/src/breakpoints_toml.rs`.
- CI boundary grep guard rejects new ad-hoc TOML parsers.

Closes ADRs: 082.

Independent of PR 1a; either order works.

### PR 2 — `engine-splatting` Level-2 crate (3DGS core)

The largest single PR in Phase 6. New crate; new asset format; new
oracle path; 3 new pixel-parity fixtures; one new importer
subprocess. Independent of PR 3 + PR 4.

- New `crates/engine-splatting/` (Level 2) with `SplatCloud` SoA
  + parallel radix sort (CPU + GPU) + composite pass + ESPL
  encoder/decoder + glTF KHR_gaussian_splatting reader.
- New WGSL shaders: `splat_sort.wgsl` + `splat_composite.wgsl`.
- New CPU oracle module `testbed/engine-raster/src/splat.rs`.
- 3 pixel-parity fixtures (`splat_sphere` strict 1/255;
  `splat_garden_1m` SSIM ≥ 0.95; `splat_view_dependent` strict
  on SH eval + SSIM on composite).
- New importer subprocess `tools/engine-splat-import/` with
  `.ply` / `.splat` / `.glb` inputs.
- Sort replay-parity oracle (CPU + GPU produce byte-identical
  permutations across worker counts).
- CI grep guard for `ply::` / `splat_format::` outside the
  importer.

Closes ADRs: 077 (3DGS architecture), 078 (ESPL + glTF reader).

### PR 3 — Vendor SDK FFI (DLSS Streamline 2.x + FSR 4 + XeSS 2)

Replaces the three vendor stub providers with real SDK-backed
implementations.

- Three vendored SDKs under
  `tools/upscaler-vendor-sdks/{streamline,fsr,xess}/` with
  per-vendor `LICENSE-VENDOR.txt` + `BLAKE3.txt` digest manifest +
  `*-sys` bindgen crate.
- Real provider implementations in
  `crates/engine-upscale-vendor/src/{dlss,fsr,xess}.rs` replacing
  stubs (cargo-feature gated; default build links no vendor SDKs).
- Loader-thread sandbox per ADR-066 §1.
- `crates/engine-upscale-vendor/Cargo.toml` — adds optional `*-sys`
  deps; `dlss / fsr / xess / all-vendors` features.
- `deny.toml` — per-vendor license fingerprint allowances.
- `docs/runbooks/vendor-upscaler-sdks.md` — fetch + verify
  procedure.
- `docs/adr/051-acknowledged-deviations.md` — entries 5, 6, 7.

Closes ADRs: 079 (vendor SDK FFI). Strengthens ADR-066 (cascade
gains real implementations).

### PR 4 — Trained v1 ONNX temporal upscaler

The neural-rendering deliverable.

- `tools/onnx-train/` — Python pipeline (pinned `requirements.txt`,
  `gen_training_data.py`, `model.py`, `train.py`, `export.py`,
  `validate_ssim.py`).
- `bin/engine-bench-frame-pacing/src/main.rs` —
  `--emit-oracle-frames` subcommand.
- `crates/engine-upscale-vendor/src/ort_temporal.rs` — real
  `ort::Session` integration; ROCm explicit-disable on Polaris
  GFX8 (x86_64 Linux); CPU AVX2 path is the user's fallback.
- `crates/engine-render/src/upscale.rs` —
  `OwnedOnnxTemporal::upscale()` routes through the vendor crate.
- `crates/engine-render/assets/onnx/temporal_upscaler_v1.onnx` —
  Git-LFS tracked.
- `.gitattributes` + `CONTRIBUTING.md` — LFS setup.
- `crates/engine-render/tests/onnx_ssim_oracle.rs` — quality test
  at the achieved SSIM bound.
- `docs/adr/067-owned-onnx-temporal-upscaler.md` — third amendment
  recording the achieved SSIM (target ≥ 0.97; floor ≥ 0.95 for
  ship-with-documented-gap).
- `docs/adr/051-acknowledged-deviations.md` — entry 4 flips from
  active token to active inference.

Closes ADRs: 080. Strengthens ADR-067 (third amendment).

### PR 5 — 3DGS frame-pacing scenes + v0.4 milestone baseline

Per ADR-070's RX 580 re-baseline + the user-approved Phase 5.5
disposition (frame-pacing gate stays local-only): no CI workflow
change. New scenes + new budgets + a v0.4 baseline measurement
landed locally.

- `testbed/frame-pacing/scenes/splat_garden_1m.ron` — the 1M-splat
  garden scene (PR 2's pixel-parity fixture re-used as a
  frame-pacing scene).
- `testbed/frame-pacing/scenes/combined_pbr_plus_splat.ron` — full
  deferred PBR + 100k-splat ambient overlay.
- `tools/frame-pacing/budgets.toml` — budget rows for the two new
  scenes (splat_garden_1m at 16.6 ms p99 = 60 FPS).
- `bin/engine-bench-frame-pacing/src/main.rs` — JSON fields:
  `splat_count`, `splat_sort_ms_p99`, `splat_composite_ms_p99`,
  `upscale_dispatch_ms_p99`, `upscaler_input_extent`,
  `upscaler_output_extent`.
- `docs/observatory/phase-6-milestone-baseline.md` — first
  measurement on the user's i7-6700 + RX 580 hardware.
- `docs/runbooks/frame-pacing-runner.md` — v0.4 baseline procedure.

No CI changes per user's Phase 5.5 disposition.

### PR 6 — Phase 6 closure (Engine Core v0.4 tag)

Pure manifest + docs PR. Lands after PRs 0–5 have all merged.

- `engine.toml` — `phase = "5.5-closed"` → `phase = "6-closed"` +
  header paragraph documenting the 7 implementation PRs.
- `README.md` — v0.4 paragraph (mirrors v0.3 paragraph structure).
- 8 closure addenda — one per design ADR (077–084).
- `docs/adr/068-phase-6-pr-slicing.md` — addendum noting ADR-084
  supersedes its Phase-6-slicing role; ADR-068 remains the Phase
  5.5 record.
- `docs/audit/oracle-exceptions.md` — sunset rows populated with
  PR 1a's commit hash.
- Memory hygiene updates.

Closes ADRs: 077, 084 (closure addenda); confirms 081, 082, 083
(implementations from earlier PRs).

v0.4 tag: post-merge user action (`git tag v0.4`).

## ADR ↔ PR matrix

```
PR 0 (design) → ADRs 077, 078, 079, 080, 081, 082, 083, 084 (this)
PR 1a         → ADR 081 (sunset + 046 amendment), ADR 083 (UpscalePass)
PR 1b         → ADR 082 (engine-config)
PR 2          → ADR 077 (3DGS arch), ADR 078 (ESPL + glTF)
PR 3          → ADR 079 (vendor SDK FFI), ADR-051 amend (entries 5-7), ADR-066 strengthen
PR 4          → ADR 080 (ONNX training), ADR-067 third amendment, ADR-051 entry 4 flip
PR 5          → none directly (populates ADR 077 §Verification)
PR 6          → ADR 077, ADR 084 closure addenda; status confirms 081, 082, 083
```

Each PR ends green `just ci`. The oracle harness gains 3 new 3DGS
fixtures; the 4 sunset entries reach strict 1/255; the 2 permanent
exceptions (cube + post_fx_chain) are documented in the *Active*
table with their categories.

## Critical files at a glance

The plan's *Critical files at a glance* table — reproduced verbatim
from the Phase 6 planning record — names the 50 files touched
across the 8 PRs. The table lives in the planning archive
(`docs/plan-archive/phase-6-radiant.md`, landed alongside PR 0)
rather than this ADR to keep the ADR concise.

## Consequences

### Positive

- Phase 6 closes with the spec's milestone literally met on the
  user's hardware.
- Each PR is independently reviewable + has a single named ADR
  set it realises.
- The cascade has real implementations at all five levels for the
  first time.
- The neural-rendering deliverable (the trained ONNX model) lands
  in tree.

### Negative

- 8 PRs is a lot of cadence for one phase. Phase 5 was 6 PRs;
  Phase 5.5 closed in 22 commits across 4 sessions; Phase 6's
  3DGS + vendor SDKs + ML pipeline expand the scope. The
  per-PR discipline (one design ADR realised; one milestone
  gate; green CI) keeps the cadence sustainable.
- PRs 3 + 4 have user-runnable steps that exceed a single review
  session — vendor SDK fetch + model training. The runbook
  documents these.

### Neutral

- ADR-068 remains the Phase 5.5 closure record. This ADR is the
  Phase 6 slicing record. The two phases are distinct ADR records
  per ADR-069's reconciliation.

## Verification

The plan is complete when:

- `just ci` green workspace-wide (build + nextest + clippy + fmt +
  deny).
- Test count climbs from 640 (Phase 5.5) to ~750+ (estimated:
  +30 from 3DGS, +10 from oracle closures, +5 from engine-config,
  +5 from ONNX SSIM oracle, +5 from vendor stubs, +5 from
  EASU/bilinear dispatch tests).
- `just frame-pacing testbed/frame-pacing/scenes/splat_garden_1m.ron`
  returns PASS on the user's RX 580 (p99 ≤ 16.6 ms = 60 FPS).
- 4 of 6 Phase 5.5 oracle exceptions sunset to strict 1/255; 2
  remain as documented permanent exceptions.
- `engine.toml` reads `phase = "6-closed"`.
- v0.4 tag applied.

## References

### Prior engine ADRs

- [ADR-053](053-phase-5-pr-slicing.md) — Phase 5 precedent.
- [ADR-068](068-phase-6-pr-slicing.md) — prior Phase 5.5
  record; stays Closed.
- [ADR-069](069-engine-spec-phase-reconciliation.md) — the
  reconciliation that defines spec Phase 6's scope.
- [ADR-077](077-3dgs-architecture.md) through
  [ADR-083](083-upscale-pass-record-discipline.md) — the design
  ADRs this slicing record orchestrates.

### Spec

- ENGINE_SPECIFICATION_v2.0.md lines 1633–1637 — the spec Phase 6
  deliverable.
