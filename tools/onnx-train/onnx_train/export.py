"""Export the best validation checkpoint to ONNX (ADR-080 §7).

ONNX opset 17 (compatible with `onnxruntime` 1.20.x via the `ort`
2.0 Rust crate). The exported artifact lands at
`crates/engine-render/assets/onnx/temporal_upscaler_v1.onnx`
(Git-LFS tracked).
"""

from __future__ import annotations

import argparse
from pathlib import Path

import torch

from .model import TemporalUpscalerV1


def export(checkpoint_path: Path, output_path: Path) -> None:
    model = TemporalUpscalerV1()
    state = torch.load(checkpoint_path, map_location="cpu", weights_only=True)
    model.load_state_dict(state["model_state_dict"])
    model.eval()

    # Dummy input matches the runtime shape: 4-frame 720p history.
    dummy = torch.randn(1, 4, 3, 720, 1280)

    output_path.parent.mkdir(parents=True, exist_ok=True)
    torch.onnx.export(
        model,
        (dummy,),
        str(output_path),
        export_params=True,
        opset_version=17,
        do_constant_folding=True,
        input_names=["history"],
        output_names=["upscaled"],
        dynamic_axes={
            "history": {0: "batch_size", 3: "height", 4: "width"},
            "upscaled": {0: "batch_size", 2: "out_height", 3: "out_width"},
        },
    )
    print(f"exported ONNX model to {output_path}")
    print(f"  source checkpoint: {checkpoint_path}")
    print(f"  checkpoint epoch:  {state.get('epoch', 'unknown')}")
    print(f"  val SSIM at save:  {state.get('val_ssim', 'unknown')}")

    # Sanity check: ONNX file is non-trivial.
    size_mb = output_path.stat().st_size / (1024 * 1024)
    print(f"  output size:       {size_mb:.2f} MiB")


def main() -> None:
    p = argparse.ArgumentParser()
    p.add_argument(
        "--checkpoint",
        type=Path,
        default=Path(__file__).parent.parent / "checkpoints/best.pt",
        help="checkpoint to export (default: ./checkpoints/best.pt).",
    )
    p.add_argument(
        "--output",
        type=Path,
        default=None,
        help="output ONNX path (default: workspace assets/onnx dir).",
    )
    args = p.parse_args()

    if args.output is None:
        # Default to the repo-root assets directory.
        workspace = Path(__file__).resolve().parents[3]
        args.output = (
            workspace / "crates/engine-render/assets/onnx/temporal_upscaler_v1.onnx"
        )

    export(args.checkpoint, args.output)


if __name__ == "__main__":
    main()
