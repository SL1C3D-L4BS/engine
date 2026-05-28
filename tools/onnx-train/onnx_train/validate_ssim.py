"""Validate the exported ONNX model's SSIM against native 1440p
(ADR-080 §6 + ADR-067 amendment).

Runs the model via onnxruntime CPU backend against the same data
shape the runtime SSIM oracle test in
`crates/engine-render/tests/onnx_ssim_oracle.rs` exercises.
"""

from __future__ import annotations

import argparse
from pathlib import Path

import numpy as np
import onnxruntime as ort
from PIL import Image
from skimage.metrics import structural_similarity as ssim


def load_history(low_dir: Path, end_idx: int, history_len: int = 4) -> np.ndarray:
    frames = []
    for h in range(history_len):
        i = end_idx - history_len + 1 + h
        arr = np.asarray(
            Image.open(low_dir / f"frame_{i:06d}.png").convert("RGB"),
            dtype=np.float32,
        ) / 255.0
        # NHWC -> NCHW.
        frames.append(np.transpose(arr, (2, 0, 1)))
    return np.expand_dims(np.stack(frames, axis=0), axis=0)


def load_target(high_dir: Path, end_idx: int) -> np.ndarray:
    arr = np.asarray(
        Image.open(high_dir / f"frame_{end_idx:06d}.png").convert("RGB"),
        dtype=np.float32,
    ) / 255.0
    return np.transpose(arr, (2, 0, 1))


def per_channel_ssim(pred: np.ndarray, target: np.ndarray) -> float:
    return float(
        np.mean(
            [
                ssim(pred[c], target[c], data_range=1.0)
                for c in range(3)
            ],
        ),
    )


def main() -> None:
    p = argparse.ArgumentParser()
    p.add_argument(
        "--model",
        type=Path,
        required=True,
        help="path to temporal_upscaler_v1.onnx.",
    )
    p.add_argument(
        "--data-dir",
        type=Path,
        default=Path(__file__).parent.parent / "data",
    )
    p.add_argument(
        "--frames",
        type=int,
        default=120,
        help="number of held-out frames to validate (default: 120 = 2s).",
    )
    p.add_argument(
        "--start-frame",
        type=int,
        default=3000,
        help="first frame index in the held-out test split.",
    )
    args = p.parse_args()

    session = ort.InferenceSession(str(args.model), providers=["CPUExecutionProvider"])

    low_dir = args.data_dir / "720p"
    high_dir = args.data_dir / "1440p"

    scores = []
    for i in range(args.frames):
        end_idx = args.start_frame + i
        history = load_history(low_dir, end_idx)
        target = load_target(high_dir, end_idx)

        outputs = session.run(["upscaled"], {"history": history})
        pred = outputs[0][0]  # (3, 1440, 2560)
        score = per_channel_ssim(pred, target)
        scores.append(score)
        if (i + 1) % 10 == 0:
            print(f"  frame {end_idx:6d} | SSIM = {score:.4f}")

    mean_ssim = float(np.mean(scores))
    min_ssim = float(np.min(scores))
    print(f"\nSSIM over {args.frames} held-out frames:")
    print(f"  mean: {mean_ssim:.4f}")
    print(f"  min:  {min_ssim:.4f}")
    print()
    if mean_ssim >= 0.97:
        print(f"✓ achieves the spec target (SSIM ≥ 0.97); ship + record in ADR-067 amendment")
    elif mean_ssim >= 0.95:
        print(f"○ within ship-with-divergence band [0.95, 0.97); ship + document gap in ADR-067")
    else:
        print(f"✗ below 0.95 floor; do not ship — re-train with adjusted hyperparameters")


if __name__ == "__main__":
    main()
