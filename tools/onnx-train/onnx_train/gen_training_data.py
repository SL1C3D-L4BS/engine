"""Render oracle training frame pairs at 720p + 1440p (ADR-080 §3).

Invokes `engine-bench-frame-pacing --emit-oracle-frames` against
the canonical `combined_deferred_scene` to render a 60-second
deterministic orbit at both internal (1280×720) and display
(2560×1440) resolutions. Saves the resulting PNGs + motion vectors
+ depth buffers under `data/`.

Use this once before training; the data is deterministic and
git-ignored.
"""

from __future__ import annotations

import argparse
import shutil
import subprocess
import sys
from pathlib import Path


def workspace_root() -> Path:
    """Locate the repo root from this script's path."""
    return Path(__file__).resolve().parents[3]


def run_oracle_emit(workspace: Path, out_dir: Path, internal_res: str, display_res: str) -> None:
    """Run the bench binary with the --emit-oracle-frames flag."""
    out_dir.mkdir(parents=True, exist_ok=True)
    cmd = [
        "cargo",
        "run",
        "--release",
        "-p",
        "engine-bench-frame-pacing",
        "--",
        "--emit-oracle-frames",
        "--scene",
        "combined_deferred_scene",
        "--internal-res",
        internal_res,
        "--display-res",
        display_res,
        "--camera-path",
        str(workspace / "testbed/frame-pacing/cameras/training_orbit.ron"),
        "--output-dir",
        str(out_dir),
    ]
    print(f"$ {' '.join(cmd)}")
    res = subprocess.run(cmd, cwd=workspace)
    if res.returncode != 0:
        sys.exit(f"oracle-emit failed (returncode {res.returncode})")


def main() -> None:
    p = argparse.ArgumentParser(
        description="Generate oracle training pairs for the v1 temporal upscaler.",
    )
    p.add_argument(
        "--frames",
        type=int,
        default=3600,
        help="number of frames to render at each resolution (default: 3600 = 60s @ 60 FPS).",
    )
    p.add_argument(
        "--data-dir",
        type=Path,
        default=Path(__file__).parent.parent / "data",
        help="output directory (default: ./data/).",
    )
    p.add_argument(
        "--clean",
        action="store_true",
        help="remove existing data/ contents before re-rendering.",
    )
    args = p.parse_args()

    workspace = workspace_root()
    data_root = args.data_dir.resolve()
    if args.clean and data_root.exists():
        for child in data_root.iterdir():
            if child.name == ".gitkeep":
                continue
            if child.is_dir():
                shutil.rmtree(child)
            else:
                child.unlink()

    print(f"workspace root: {workspace}")
    print(f"data root:     {data_root}")
    print(f"frames per res: {args.frames}")

    # 720p (the upscaler's input).
    run_oracle_emit(
        workspace,
        data_root / "720p",
        internal_res="1280x720",
        display_res="2560x1440",
    )
    # 1440p (the supervision target).
    run_oracle_emit(
        workspace,
        data_root / "1440p",
        internal_res="2560x1440",
        display_res="2560x1440",
    )

    print(f"oracle frame generation complete; pairs land under {data_root}")


if __name__ == "__main__":
    main()
