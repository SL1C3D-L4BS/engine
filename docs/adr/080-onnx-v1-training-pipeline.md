# ADR-080 — ONNX temporal upscaler v1 training pipeline

- Status: Accepted (Phase 6 design contract; implementation lands in
  Phase 6 PR 4)
- Date: 2026-05-28
- Phase: 6 — NEURAL RENDERING & GAUSSIAN SPLATTING
- Companion: ADR-005 (vendor upscaler trait), ADR-046 (rasterizer
  oracle regression — the training reference), ADR-051 (acknowledged
  deviations — entry 4 ORT flips active inference), ADR-067 (Owned
  ONNX temporal upscaler — parent contract), ADR-084 (Phase 6 PR
  slicing)

## Context

ADR-067 named `OwnedOnnxTemporal` as the engine's universal-coverage
upscaler and ADR-051 entry 4 acknowledged the ORT runtime
dependency. Phase 5.5 A.4 / A.5 (commit `08f6bd9`) shipped the
provider's *cascade behaviour* — `supports() = true` always; the
runtime emits the cascade-selected token and falls back to the CPU
oracle's bilinear when the `ort-runtime` cargo feature is off. The
*actual ONNX model* (`temporal_upscaler_v1.onnx`) and the *real
inference path* using `ort::Session` are not yet in tree.

Phase 6 closes that gap. The training pipeline produces a v1 model
trained against the engine's CPU oracle reference frames; the
runtime loads the bundled model and performs real inference via the
ORT crate's CPU AVX2 backend (the user's hardware tier — i7-6700
Skylake; no CUDA; ROCm is explicitly disabled on Polaris GFX8 per
the user's hardware not supported by ROCm 5+).

The training pipeline is *out of the engine binary*: it lives in a
separate Python tool (`tools/onnx-train/`) using PyTorch +
torchvision + onnxruntime. This matches ADR-067 §1:

> the training pipeline is out of scope for the engine repo and
> lives in a separate dev tool

with the dev tool now in-tree under `tools/` (the dev tool *is*
part of the repo; it is just isolated from the engine binary).

## Decision

### 1. `tools/onnx-train/` layout

```
tools/onnx-train/
├── README.md                # invocation guide
├── requirements.txt         # pinned Python deps
├── onnx_train/
│   ├── __init__.py
│   ├── gen_training_data.py # render oracle pairs at 720p + 1440p
│   ├── model.py             # PyTorch model architecture
│   ├── train.py             # training loop
│   ├── export.py            # PyTorch → ONNX export
│   └── validate_ssim.py     # SSIM validation against oracle
├── data/                    # generated frame pairs (gitignored except for .gitkeep)
│   └── .gitkeep
└── checkpoints/             # training checkpoints (gitignored)
    └── .gitkeep
```

### 2. Pinned Python dependencies (`requirements.txt`)

```
torch==2.5.1
torchvision==0.20.1
numpy==2.0.2
Pillow==10.4.0
onnx==1.17.0
onnxruntime==1.20.1
scikit-image==0.24.0
tqdm==4.66.5
```

Pins are exact. The pipeline is not expected to track the
PyTorch ecosystem's rolling release; reproducibility wins over
freshness. The CI lane that exercises the pipeline (if any) builds
a fresh venv from this file.

### 3. Training data generation

`gen_training_data.py` invokes the engine's bench binary in
"oracle frame" mode:

```bash
cargo run --release -p engine-bench-frame-pacing -- \
    --emit-oracle-frames \
    --scene combined_deferred_scene \
    --internal-res 1280x720 \
    --display-res 2560x1440 \
    --camera-path testbed/frame-pacing/cameras/training_orbit.ron \
    --output-dir tools/onnx-train/data/
```

The bench renders 60 seconds × 60 FPS = 3600 frame pairs along a
deterministic camera orbit. Each pair includes:

- `data/720p/frame_NNNNNN.png` — 1280×720 LDR sRGB
- `data/720p/motion_NNNNNN.bin` — 1280×720 × 2 channel f32
  motion vectors
- `data/720p/depth_NNNNNN.bin` — 1280×720 × 1 channel f32 linear
  depth
- `data/1440p/frame_NNNNNN.png` — 2560×1440 LDR sRGB (target)

The bench's `--emit-oracle-frames` flag is added in PR 4 (a small
addition to `bin/engine-bench-frame-pacing/src/main.rs`); the
camera-path RON file is committed under
`testbed/frame-pacing/cameras/training_orbit.ron`.

### 4. Model architecture (`model.py`)

A compact CNN with temporal attention + sub-pixel convolution
upsampling:

```python
class TemporalUpscalerV1(nn.Module):
    def __init__(self):
        # Backbone: 3 conv blocks with residual connections
        #   - conv 3→64, ReLU, conv 64→64, ReLU
        #   - conv 64→128, ReLU, conv 128→128, ReLU
        #   - conv 128→256, ReLU
        #
        # Temporal attention over 4-frame trailing history:
        #   - LSTM-style accumulator (Goodfellow Ch. 10)
        #   - input: current frame embeddings + 3 prior frame embeddings
        #   - output: temporally-pooled embeddings
        #
        # Sub-pixel convolution upsampling (Shi et al. 2016 ESPCN):
        #   - conv 256→ (4 × 3 = 12) at internal res
        #   - PixelShuffle(2) → output res, 3 channels
```

References:

- Goodfellow Ch. 9 — convolutional networks (the backbone)
- Goodfellow Ch. 10 — sequence modeling (the temporal attention)
- Goodfellow Ch. 14 — autoencoders (the conceptual frame)
- Shi et al. 2016 — *Real-Time Single Image and Video
  Super-Resolution Using an Efficient Sub-Pixel Convolutional
  Neural Network* (the sub-pixel conv upsampling)

The model has ~3 M parameters. ONNX-exported size: ~3 MiB.

### 5. Training loop (`train.py`)

- **Optimizer**: AdamW, learning rate 3e-4, weight decay 1e-4
- **Schedule**: cosine annealing over 100 epochs
- **Loss**: L1 reconstruction + 0.1 × VGG-19 perceptual loss
  (mid-layer features)
- **Batch**: 8 (memory-bounded on the user's hardware; the script
  honours `CUDA_VISIBLE_DEVICES` but defaults CPU AVX2 if no GPU)
- **Checkpoint**: every 10 epochs to `checkpoints/`; best by SSIM

Per-epoch logging includes the running L1 + perceptual losses + a
validation-set SSIM. The training run produces a `training.log`
text file the runbook references.

### 6. Quality target + amendment clause

Per ADR-067 §6's universal-coverage promise, the v1 model must
beat OwnedBilinear meaningfully. Spec line 1634 names "high
quality" without a numeric SSIM target; the design target is
**SSIM ≥ 0.97 vs native 1440p**.

If validation achieves the target, ADR-067 gets an amendment that
records the achieved bound (e.g. "v1 ships at SSIM 0.972 measured
against the canonical orbit").

If validation lands at **SSIM in [0.95, 0.97)**, the v1 model
*still ships* (it is meaningfully better than bilinear; bilinear's
SSIM on the same orbit is ~0.82). The ADR-067 amendment documents
the achieved value as the v0.4-baseline quality and names the
quality gap to the spec target. This is *not* a deferral: v1
ships, with documented divergence, per the "zero gated" Phase 6
discipline.

If validation lands at **SSIM < 0.95**, the v1 model does *not*
ship; the training run is rejected and the pipeline iterates
(adjust hyperparameters, regenerate data with a different camera
path, augment the loss). The 0.95 floor is the engineering line
below which the model is worse than the cube fixture's parity
floor (max_delta 0.0055 linear ≈ SSIM 0.99 on equivalent content;
0.95 is a generous margin below that).

### 7. ONNX export (`export.py`)

The best checkpoint by validation SSIM is exported via PyTorch's
`torch.onnx.export()` at ONNX opset 17 (compatible with
`onnxruntime` 1.20.x via the `ort` 2.0 Rust crate). The exported
artifact is written to
`crates/engine-render/assets/onnx/temporal_upscaler_v1.onnx`.

The export validates: a forward pass through the exported ONNX
model on a held-out batch must match the PyTorch reference within
1e-5 L1.

### 8. Git LFS

`crates/engine-render/assets/onnx/*.onnx` is Git-LFS tracked.
The first LFS asset in the repo (per ADR-067 §Consequences):

```
# .gitattributes
crates/engine-render/assets/onnx/*.onnx filter=lfs diff=lfs merge=lfs -text
```

`CONTRIBUTING.md` grows a `git lfs install` step in the contributor
setup section. The training data + checkpoints stay in
`.gitignore`'d directories — only the *exported* model file is
tracked.

### 9. SSIM validation oracle

`crates/engine-render/tests/onnx_ssim_oracle.rs` is a Rust test
that:

1. Loads the bundled `temporal_upscaler_v1.onnx`.
2. Runs inference on a stored fixture (one 720p oracle frame +
   3-frame history) via the `ort` runtime.
3. Computes SSIM against the corresponding 1440p oracle frame.
4. Asserts SSIM ≥ {achieved bound from ADR-067 amendment}.

The fixture is committed under
`crates/engine-render/tests/fixtures/onnx_ssim/` (single 720p frame
+ 3 prior frames + 1 reference 1440p frame, totalling ~12 MB,
**not** LFS-tracked — these are PNG snapshots, small enough for
plain git).

### 10. ROCm explicit-disable on Polaris

The `ort` crate's execution-provider cascade tries ROCm before
CPU on Linux x86_64. AMD dropped ROCm support for Polaris GFX8
after ROCm 5.x; the user's RX 580 has no ROCm path. The runtime
provider initialisation explicitly skips ROCm on x86_64 Linux:

```rust
// crates/engine-upscale-vendor/src/ort_temporal.rs

fn build_session(model_path: &Path) -> ort::Result<ort::Session> {
    use ort::execution_providers::*;

    let mut providers: Vec<ExecutionProviderDispatch> = vec![];
    #[cfg(target_os = "windows")]
    providers.push(DirectMLExecutionProvider::default().into());
    #[cfg(target_os = "macos")]
    providers.push(CoreMLExecutionProvider::default().into());
    // ROCm explicitly skipped: AMD dropped GFX8 (Polaris) after ROCm 5.x.
    // The CPU AVX2 path is the user's hardware fallback; ~3-6 ms per frame
    // at 720p→1440p inference on i7-6700 within the 16.6 ms frame budget.
    providers.push(CPUExecutionProvider::default().into());

    ort::Session::builder()?
        .with_execution_providers(providers)?
        .commit_from_file(model_path)
}
```

The session is built lazily on the first `upscale()` call. Failure
to initialise (e.g. ORT binaries missing) downgrades the provider
to bilinear via the cascade.

## Consequences

### Positive

- The neural-rendering deliverable is realised end-to-end: a model
  exists, trains against the engine's oracle, exports to ONNX,
  loads through ORT, and runs per-frame on the user's hardware.
- The training pipeline is reproducible and the model's quality is
  measured (SSIM oracle test in CI).
- The ROCm explicit-disable closes a known footgun on Polaris
  hardware; the CPU AVX2 fallback is the correct path for the
  user's tier.

### Negative

- The training step is a one-shot user-runnable script (hours of
  wall-clock on CPU). The trained model artifact ships in tree via
  Git LFS so the engine binary's per-frame inference does *not*
  require Python or a training run. The runbook documents the
  retraining workflow.
- ADR-067 must be amended a *third* time (the first two amendments
  preceded; this one records the achieved SSIM bound). The ADR's
  status remains Accepted; amendments are documented per ADR-067's
  precedent.

### Neutral

- The `data/` + `checkpoints/` directories are gitignored. Only
  the exported model artifact + the SSIM-fixture PNGs are tracked.

## Implementation

PR 4 of Phase 6 (per ADR-084):

1. `tools/onnx-train/` directory + Python pipeline.
2. `bin/engine-bench-frame-pacing/src/main.rs` — `--emit-oracle-frames`
   subcommand.
3. `testbed/frame-pacing/cameras/training_orbit.ron` — the
   deterministic camera path.
4. `crates/engine-upscale-vendor/src/ort_temporal.rs` — real
   `ort::Session` integration replacing the token stub.
5. `crates/engine-upscale-vendor/Cargo.toml` — `ort = { version =
   "2.0", optional = true }` behind `ort-runtime` feature.
6. `crates/engine-render/src/upscale.rs` — `OwnedOnnxTemporal::upscale()`
   routes through the vendor crate's real inference path.
7. `crates/engine-render/tests/onnx_ssim_oracle.rs` — quality test.
8. `crates/engine-render/assets/onnx/temporal_upscaler_v1.onnx`
   (Git LFS).
9. `.gitattributes` + `CONTRIBUTING.md` LFS setup.
10. `docs/adr/067-owned-onnx-temporal-upscaler.md` — third
    amendment recording the achieved SSIM bound.

## References

### Papers

- Shi, W. et al. (2016). *Real-Time Single Image and Video
  Super-Resolution Using an Efficient Sub-Pixel Convolutional
  Neural Network*. CVPR 2016.
  <https://arxiv.org/abs/1609.05158>.
- Wang, Z. et al. (2004). *Image Quality Assessment: From Error
  Visibility to Structural Similarity* (SSIM). IEEE TIP.
  <https://ece.uwaterloo.ca/~z70wang/publications/ssim.pdf>.

### Books

- *Deep Learning* (Goodfellow / Bengio / Courville) — Ch. 9 (CNNs),
  Ch. 10 (Sequence Modeling), Ch. 14 (Autoencoders).

### Runtime

- ORT Rust crate — <https://github.com/pykeio/ort>.
- ONNX Runtime — <https://onnxruntime.ai/>.

### Prior engine ADRs

- [ADR-005](005-upscaler-provider-trait.md) — trait surface.
- [ADR-046](046-rasterizer-oracle-regression.md) — the training
  reference frames come from the oracle harness.
- [ADR-051](051-acknowledged-deviations.md) — entry 4 (ORT) flips
  from active token to active inference.
- [ADR-067](067-owned-onnx-temporal-upscaler.md) — the parent
  ADR; gains a third amendment via this implementation.
- [ADR-084](084-phase-6-pr-slicing.md) — Phase 6 PR slicing.
