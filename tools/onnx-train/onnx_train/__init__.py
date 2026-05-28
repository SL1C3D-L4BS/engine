"""ONNX temporal upscaler v1 training pipeline (ADR-080).

Submodules:
    gen_training_data — render oracle frame pairs at 720p + 1440p.
    model — TemporalUpscalerV1 architecture (CNN + temporal +
        sub-pixel conv).
    train — training loop (AdamW + cosine LR + L1+perceptual loss).
    export — PyTorch -> ONNX export.
    validate_ssim — held-out SSIM validation against native 1440p.
"""

__all__ = [
    "gen_training_data",
    "model",
    "train",
    "export",
    "validate_ssim",
]

# Version-stamped for reproducibility tracing.
__version__ = "1.0.0"
