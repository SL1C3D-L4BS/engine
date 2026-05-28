# `temporal_upscaler_v1.onnx` training pipeline (ADR-080)

PyTorch training pipeline for the engine's owned ONNX temporal
upscaler. Outputs `temporal_upscaler_v1.onnx` consumed by the
runtime per ADR-067.

## Quick start

```sh
# 1. Create venv + install pinned deps.
python -m venv .venv && source .venv/bin/activate
pip install -r requirements.txt

# 2. Generate oracle training pairs (renders 3600 frames at 720p
#    + 1440p along a deterministic camera path; ~10 min wall-clock
#    on i7-6700 + RX 580 with the cargo-bench oracle-emit flag).
python -m onnx_train.gen_training_data

# 3. Train (CNN backbone + temporal attention + sub-pixel conv
#    upsampling; 100 epochs default; ~12 hours on CPU AVX2,
#    ~3 hours on a single GPU).
python -m onnx_train.train

# 4. Export the best validation checkpoint to ONNX.
python -m onnx_train.export

# 5. Validate SSIM against the held-out test split.
python -m onnx_train.validate_ssim
```

The exported artifact lands at
`crates/engine-render/assets/onnx/temporal_upscaler_v1.onnx`
(Git-LFS tracked).

## Pipeline outputs

| File | Purpose |
|---|---|
| `data/{720p,1440p}/frame_*.png` | Rendered oracle frame pairs. |
| `data/{720p,1440p}/{motion,depth}_*.bin` | Per-frame motion vectors + linear depth. |
| `checkpoints/epoch_*.pt` | PyTorch checkpoints (every 10 epochs). |
| `training.log` | Per-epoch L1 / perceptual / SSIM trace. |
| `temporal_upscaler_v1.onnx` | Final exported model (committed via LFS). |

## Quality target + SSIM bound

Per ADR-080 §6: the spec's design target is **SSIM ≥ 0.97 vs native
1440p**. If validation reaches the target, `temporal_upscaler_v1.onnx`
ships with the achieved SSIM recorded in ADR-067's amendment. If
validation lands in **[0.95, 0.97)**, the model still ships and the
amendment names the achieved value as the v0.4 baseline quality.
If validation falls below 0.95, the model does not ship; the
hyperparameters / training data are revisited.

## Runtime side

The Rust runtime is in `crates/engine-upscale-vendor/src/ort_temporal.rs`
(behind the `ort-runtime` cargo feature). The session-init path
explicitly disables ROCm on Polaris GFX8 (the user's RX 580) per
ADR-080 §10; the CPU AVX2 backend is the user's fallback.

## References

- ADR-067 — `OwnedOnnxTemporal` provider (parent).
- ADR-080 — this training pipeline.
- ADR-051 entry 4 — ORT runtime acknowledged-deviation register
  entry (active inference, post-Phase 6 PR 4).
