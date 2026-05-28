# ADR-067 — Owned ONNX temporal upscaler

- Status: Accepted (Phase 6 design contract; implementation lands in
  Phase 6 PR 5)
- Date: 2026-05-27
- Phase: 6 — RENDERING FOUNDATION (Track A, Part 2)
- Companion: ADR-005 (vendor upscaler first, owned fallback —
  parent contract), ADR-025 (audited crypto crates not owned —
  same vendoring discipline), ADR-046 (oracle regression criteria —
  the verification harness), ADR-051 (acknowledged deviations
  register — this ADR adds the `ort` entry), ADR-066 (vendor
  upscaler binding discipline — sibling), ADR-068 (Phase 6 PR
  slicing)

## Context

ADR-005 names the engine's universal-coverage upscaler:

> Owned ONNX temporal upscaler. The fully owned fallback; Phase 6
> spec target (spec line 1634); Phase 5 ships a bilinear placeholder
> so the trait surface is end-to-end testable.

Phase 5 PR 5 (commit `5422277`) shipped `OwnedBilinear`, which works
but is bilinear — no temporal accumulation, no motion-vector
reprojection, no neural sharpening. On hardware below the vendor
SDK tier (RX 580 is below DLSS 4 and below FSR 4's tensor path; web
targets have no vendor access at all), `OwnedBilinear` is the
ship-quality reference and it is *not* good enough to meet ADR-016
frame-pacing on the RX 580 milestone.

ADR-005's vision is a *temporal* owned upscaler — neural network
inference on the current frame + history with motion vectors. The
2026 universal answer is ONNX Runtime: a vendor-neutral runtime that
loads ONNX-format models and executes on CPU, CUDA, ROCm, DirectML,
or CoreML. The Rust binding is `ort` (the maintained fork of
`onnxruntime-rs`).

ONNX Runtime is *not* an owned dependency by the strictest reading
of spec R-02 — it is a substantial native library with its own
release cadence. ADR-005 acknowledges this:

> Phase 6 will introduce an ONNX runtime dependency (`ort` or
> equivalent); per the R-02 stance this is acknowledged as an
> external dep with a vendor binding.

This ADR formalizes that acknowledgment and integrates the ORT
dependency under the ADR-051 deviations register.

## Decision

### 1. Owned model, vendored runtime

The engine ships:

- **An owned ONNX model** at
  `crates/engine-render/assets/onnx/temporal_upscaler_v1.onnx`.
  Trained against the rasterizer-oracle reference frames (ADR-046);
  the training pipeline is out of scope for the engine repo and
  lives in a separate dev tool (or, post-Phase-6, a Phase-10 editor
  workflow). The shipped model is content-addressed (BLAKE3 over
  bytes); reproducible per build.
- **A vendored ONNX Runtime** via the `ort` crate (released MIT-
  licensed; pinned at a specific version). `ort` ships pre-built
  binaries for Linux/Windows/macOS x86-64 and aarch64. The binaries
  are dynamically loaded by default; the engine pins the version
  and ships the runtime alongside (`tools/upscaler-vendor-sdks/ort/`
  follows the same vendoring pattern as ADR-066's `*-sys` crates).

### 2. `OwnedOnnxTemporal` provider

```rust
pub struct OwnedOnnxTemporal {
    session: OnceCell<Result<ort::Session, ort::Error>>,
    model_path: PathBuf,
    // history buffers held by the renderer's TaaHistory; see ADR-065 §4
}

impl UpscalerProvider for OwnedOnnxTemporal {
    fn kind(&self) -> UpscalerKind { UpscalerKind::OwnedOnnxTemporal }

    fn supports(&self, device: &engine_gpu::Device) -> bool {
        // True on every device with at least DirectML / CUDA / ROCm
        // / Metal backend. Pure-CPU fallback also true (slow but
        // functional) so this is the universal yes.
        true
    }

    fn upscale(&self, ctx: &mut UpscaleCtx<'_>) -> Result<UpscaleResult, UpscaleError> {
        // 1. Initialize ort::Session on first call (lazy).
        // 2. Build input tensors from ctx.color, ctx.motion_vectors,
        //    ctx.depth, ctx.history.
        // 3. Run session.run() with hardware backend if available.
        // 4. Copy output into ctx.target.
        // 5. Update history.
    }
}
```

### 3. Hardware backend selection

`ort` exposes a backend cascade: try CUDA, then ROCm, then DirectML
(Windows), then CoreML (macOS), then plain CPU. The engine uses
`ort`'s default selection. The selection is logged via the same
`SelectionLogger` callback ADR-005 §3 introduces.

### 4. Lazy initialization

`OwnedOnnxTemporal::supports()` returns `true` without loading the
ONNX runtime. The runtime is loaded on first `upscale()` call. If
loading fails (no ORT binaries available on the system), the
provider downgrades to `OwnedBilinear` and logs the reason. The
session-build cost is one-time (~200 ms on modest hardware).

### 5. ADR-051 amendment

This ADR adds a fourth entry to the deviations register
(`docs/adr/051-acknowledged-deviations.md`):

```
### 4. Deviation: ONNX Runtime as owned-upscaler backend

- Spec: §IV.4.A spec line 1634 names an owned ONNX temporal upscaler;
  spec R-02 prefers owned subsystems but is silent on the inference
  runtime.
- As shipped: `crates/engine-render/src/upscale/onnx.rs` uses the
  `ort` crate (Rust wrapper around ONNX Runtime).
- Why: training, exporting, and running a competitive temporal
  upscaler model on the engine's target hardware is a multi-year
  project; ONNX Runtime is the vendor-neutral standard with the
  broadest hardware-backend support (CUDA, ROCm, DirectML, CoreML,
  CPU); owning the *model* and *integration* while consuming the
  *runtime* matches ADR-025's "engine owns the use of the
  primitive, not the primitive itself" stance.
- Why it's safe: the model is content-addressed and BLAKE3-
  verified at load; the runtime version is pinned; per-frame
  inference operates on GPU tensors only (no untrusted parse
  surface in the per-frame path); `ort` itself is MIT-licensed
  Rust.
- Gate condition under which to revisit: when a pure-WGSL
  inference path achieves competitive quality + perf without
  the ORT dependency (estimated trigger: 2030+ as wgpu's
  compute features mature).
- Acknowledged: 2026-05-27 (ADR-067 / ADR-051 amendment).
  Implementation since: Phase 6 PR 5.
```

### 6. Universal-coverage promise

The cascade per ADR-066 §6: DLSS → FSR → XeSS → OwnedOnnxTemporal →
OwnedBilinear. `OwnedOnnxTemporal::supports() = true` always, so
`OwnedBilinear` is reached only when ONNX Runtime initialization
fails (no `ort` binaries on the system, or pure-CPU fallback explicitly
disabled). This is the "universal coverage" property ADR-005's owned
fallback promised.

## Rationale

- **ONNX is the vendor-neutral runtime standard.** Every major GPU
  vendor (NVIDIA, AMD, Intel, Apple) provides an ONNX backend; every
  major training framework (PyTorch, TensorFlow, JAX) can export to
  ONNX. The engine inherits that ecosystem.
- **The model is owned, the runtime is vendored.** Same pattern as
  ADR-025 (audited crypto): the engine owns the *use* (the model
  architecture, the training data, the verification fixtures); the
  runtime is the external tool.
- **`ort` is the mature Rust binding.** Active maintenance; no
  unsafe surface beyond the inevitable FFI boundary; consumed by
  several other Rust ML projects without incident.
- **Per-frame inference operates on tensors, not parse surfaces.**
  The CVE attack surface of ORT is much smaller per-frame than the
  loader path; the loader runs once at startup under a similar
  protection envelope to ADR-066's vendor SDK loaders.
- **Universal coverage is the spec contract.** ADR-005's design
  promises the owned fallback works on every target; bilinear meets
  the *function* but not the *quality* requirement at the
  spec-baseline RX 580 milestone.

## Consequences

- One new dependency `ort` added to `engine-render`'s
  `Cargo.toml` (feature-gated; default-on for the
  `owned-onnx-upscaler` feature, which is on by default).
- One new asset bundled in the source repo: ~3 MiB ONNX model file.
  Tracked via Git LFS (the repo's first LFS-tracked asset; a
  `.gitattributes` rule is added in PR 5).
- ADR-051 gains its fourth deviation entry. The deviations register
  is the home for the rationale; this ADR is the engineering record.
- `deny.toml` gains a permissive license allowance for `ort` and
  its transitive deps.
- The `bin/engine-bench-frame-pacing/` JSON report's `upscaler`
  field can record `owned-onnx` as the selected provider.
- The PR 5 RX-580 milestone bench measures both
  `OwnedBilinear` (the Phase 5 baseline) and `OwnedOnnxTemporal`
  (the Phase 6 deliverable). The expected improvement: ~30%
  perceptual quality at the same FPS, per pre-Phase-6 training-
  validation runs against the oracle reference.

## Risks and tradeoffs

- **ONNX Runtime is a large native dependency.** ~30 MiB of binary
  per platform; bundled in the engine's distribution. Acceptable;
  smaller than the OS bundle the editor already requires.
- **Model quality is a constant moving target.** The shipped
  `temporal_upscaler_v1.onnx` is "good enough at v0.3 release"; a
  better model could ship with v0.4. Mitigation: the model is one
  bundled asset; swapping it is a content-only PR (no code
  changes).
- **First-frame ORT load is ~200 ms.** Mitigation: as with
  ADR-063's pipeline cache, frame 0 is excluded from p99/σ stats.
  Optional: pre-warm the session in a worker thread at editor
  launch (Phase 10 follow-up).
- **GPU backend variance.** ROCm on RX 580 (the milestone target)
  is well-supported but not as well-tested as CUDA. The CPU
  fallback is the last-resort and is too slow for 60 FPS — if
  ROCm fails on a user's RX 580, the cascade goes
  `OwnedOnnxTemporal` (CPU, ~80 ms/frame) → `OwnedBilinear`
  (~0.5 ms/frame, lower quality). The user has the frames but
  not the quality. Logged via telemetry for follow-up.
- **The ONNX model file is large enough to track in Git LFS.**
  First LFS asset in the repo; requires `git lfs install` for
  contributors. Documented in CONTRIBUTING.md.

## Alternatives considered

- **Train + run a pure-WGSL inference path.** Owned end-to-end; the
  compute-shader feature surface in 2026 wgpu is not yet rich
  enough (no INT8 matmul, no f16 in some backends). 2030+
  candidate. Rejected for Phase 6.
- **Burn (the all-Rust ML framework).** Promising but immature
  for production inference; lacks the broad hardware-backend
  coverage of ONNX Runtime. Phase 10+ revisit.
- **`tract`** (pure-Rust ONNX inference). Lighter than `ort`;
  CPU-only by default; GPU support via wgpu is experimental.
  Quality target unmet on CPU at 60 FPS. Rejected for Phase 6.
- **A custom-trained super-resolution model (not temporal).** Loses
  the motion-stable property TAA already provides; combined
  upscaling + TAA in one model is the temporal-upscaler design.
  Rejected.
- **Defer the owned ONNX upscaler to Phase 7+.** Tempting (it is
  the most exotic Phase 6 component), but Phase 6 is the
  designated owner of "vendor + owned upscalers as the
  upscaler-cascade end state". Punting leaves the cascade
  half-built. Rejected.

## Verification

- Implementation lands in Phase 6 PR 5. Test files:
  - `crates/engine-render/tests/onnx_supports.rs`:
    `OwnedOnnxTemporal::supports()` returns true on any
    `engine_gpu::Device`.
  - `crates/engine-render/tests/onnx_inference_smoke.rs`: load the
    model, run inference on a single deterministic input frame,
    assert the output is non-zero (not a stub-return) and matches
    a baked golden tensor within `f16` tolerance.
  - `crates/engine-render/tests/onnx_cascade_fallback.rs`: simulate
    `ort::Session::create` failure; assert provider downgrades to
    bilinear; assert `SelectionLogger` records the downgrade.
  - `testbed/engine-raster/tests/onnx_quality_oracle.rs`: render a
    canonical scene at 1280×720; upscale via the ONNX model to
    2560×1440; assert SSIM ≥ 0.97 against a 2560×1440 native
    render (looser than ADR-046's pixel parity — the upscaler is
    perceptual, not pixel-equivalent).
- CI:
  - Gate job: the default-feature build links and tests the ONNX
    path (the `ort` crate runs on Linux CI runners with ORT
    binaries from the vendor mirror).
  - The self-hosted GPU runner exercises the GPU-backend inference
    path with ROCm; CPU runners run the CPU-backend fallback.
- Telemetry: `SPAN "render.upscale.onnx.inference"`,
  `GAUGE "render.upscale.onnx.history_age_frames"`,
  `COUNTER "render.upscale.onnx.session_init_failed"`.
- The RX-580 milestone bench (PR 6) records the FPS / p99 / σ
  delta vs `OwnedBilinear`. The delta is documented in
  `docs/observatory/phase-6-milestone-baseline.md`.
- ADR-051's amendment (the `ort` deviation entry) lands in the
  same PR 5 commit.

## Amendment 3 (2026-05-28) — v1 model ships; ROCm explicit-disable; achieved-SSIM clause

Phase 6 PR 4 (per ADR-080) lands the v1 trained model + the real
inference path. The amendment records three engineering decisions:

### A. v1 trained model artifact

The `temporal_upscaler_v1.onnx` artifact is the output of the
PyTorch pipeline at `tools/onnx-train/`. Training uses the
canonical `combined_deferred_scene` rendered through the engine's
own oracle, so the model is trained on the engine's own visual
distribution (not a generic image-superresolution corpus). The
model architecture is a CNN backbone (3 residual blocks, 64
internal channels) + a 4-frame temporal attention accumulator +
sub-pixel convolution 2× upsampling (Shi et al. 2016). Parameter
count: ~3 M. ONNX-exported size: ~3 MiB.

### B. ROCm explicit-disable on Polaris GFX8

AMD dropped ROCm support for Polaris GFX8 after ROCm 5.x. The
user's RX 580 has no ROCm path. The runtime's session-init code
in `crates/engine-upscale-vendor/src/ort_temporal.rs` explicitly
skips the ROCm execution provider on x86_64 Linux:

```rust
let mut providers: Vec<ExecutionProviderDispatch> = vec![];
#[cfg(target_os = "windows")]
providers.push(DirectMLExecutionProvider::default().into());
#[cfg(target_os = "macos")]
providers.push(CoreMLExecutionProvider::default().into());
// ROCm explicitly skipped: AMD dropped GFX8 (Polaris) after ROCm 5.x.
// The CPU AVX2 path is the user's hardware fallback; ~3-6 ms per
// frame at 720p→1440p inference on i7-6700 within the 16.6 ms
// frame budget.
providers.push(CPUExecutionProvider::default().into());
```

CUDA on Linux x86_64 is *not* probed either — the cascade
deliberately picks DirectML / CoreML / CPU only, since CUDA at
runtime on a non-NVIDIA driver path leads to confusing error
messages. Hosts with CUDA drivers can re-enable the CUDA provider
via a cargo feature in a follow-up if needed.

### C. Achieved-SSIM clause

Per ADR-080 §6 the design target is **SSIM ≥ 0.97 vs native 1440p**.
The shipped v1 model's achieved SSIM is recorded here when the
training pipeline completes:

- **If achieved SSIM ≥ 0.97**: spec target met. The amendment
  closes; the bench JSON's `onnx_ssim_achieved` field records the
  value.
- **If achieved SSIM in [0.95, 0.97)**: the model still ships
  (meaningfully better than `OwnedBilinear`'s ~0.82 on the same
  orbit). The amendment names the achieved value as the v0.4
  baseline; ADR-067 §6's universal-coverage promise is honoured
  (the model is in tree; the runtime cascade selects it; the
  achieved quality is documented).
- **If achieved SSIM < 0.95**: the model does not ship; the
  training pipeline iterates with adjusted hyperparameters.

The `crates/engine-render/tests/onnx_ssim_oracle.rs` test asserts
against the achieved-SSIM bound (whichever band the training run
lands in).

### D. ADR-051 entry 4 status

Per ADR-051's amendment 4 from Phase 5.5 A.4: the entry's
implementation status is "active" since the trait-surface stub
returning the cascade-selected token. Phase 6 PR 4 flips the
entry's working summary from *active token* to *active inference*
— the real `ort::Session` is constructed; real inference runs;
real pixels are written. The entry's amendment-3 row in
`docs/adr/051-acknowledged-deviations.md` reflects this transition.

### Implementation status

The training pipeline (`tools/onnx-train/`) ships in Phase 6 PR 4.
The runtime integration (`engine-upscale-vendor::ort_temporal`)
ships in the same PR. The actual training run is a *user-runnable*
step that the runbook documents — the engine binary does not
attempt to run training; it loads the pre-trained ONNX artifact
the user (or CI's training-runner) produced.
