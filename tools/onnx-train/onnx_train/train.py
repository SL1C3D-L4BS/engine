"""Train TemporalUpscalerV1 (ADR-080 §5).

AdamW + cosine LR schedule + L1 reconstruction loss + 0.1 ×
perceptual VGG-19 loss. 100 epochs default. Best-by-SSIM checkpoint
is preserved for the export step.
"""

from __future__ import annotations

import argparse
import logging
from pathlib import Path

import torch
import torch.nn.functional as F
import torchvision.models as tvm
from torch import nn
from torch.optim import AdamW
from torch.optim.lr_scheduler import CosineAnnealingLR
from torch.utils.data import DataLoader, Dataset

from .model import TemporalUpscalerV1, parameter_count


class OracleFrameDataset(Dataset):
    """Loads 4-frame histories + the matching 1440p supervision."""

    def __init__(self, low_dir: Path, high_dir: Path, history_len: int = 4) -> None:
        super().__init__()
        self.low_dir = low_dir
        self.high_dir = high_dir
        self.history_len = history_len
        # Discover frame indices by listing low_dir.
        self.frame_indices = sorted(
            int(p.stem.split("_")[-1])
            for p in low_dir.glob("frame_*.png")
        )
        # The first (history_len - 1) frames have insufficient
        # history; skip them.
        self.frame_indices = self.frame_indices[history_len - 1 :]

    def __len__(self) -> int:
        return len(self.frame_indices)

    def __getitem__(self, idx: int) -> tuple[torch.Tensor, torch.Tensor]:
        # Lazy import — PIL is heavy.
        from PIL import Image
        import numpy as np

        end_idx = self.frame_indices[idx]
        history_frames = []
        for h in range(self.history_len):
            i = end_idx - self.history_len + 1 + h
            path = self.low_dir / f"frame_{i:06d}.png"
            arr = np.asarray(Image.open(path).convert("RGB"), dtype=np.float32) / 255.0
            history_frames.append(torch.from_numpy(arr).permute(2, 0, 1))
        history = torch.stack(history_frames, dim=0)  # (4, 3, H, W)

        target_path = self.high_dir / f"frame_{end_idx:06d}.png"
        target_arr = np.asarray(Image.open(target_path).convert("RGB"), dtype=np.float32) / 255.0
        target = torch.from_numpy(target_arr).permute(2, 0, 1)  # (3, H, W)
        return history, target


class PerceptualLoss(nn.Module):
    """Mid-layer VGG-19 perceptual loss."""

    def __init__(self) -> None:
        super().__init__()
        weights = tvm.VGG19_Weights.IMAGENET1K_V1
        vgg = tvm.vgg19(weights=weights).features.eval()
        for p in vgg.parameters():
            p.requires_grad_(False)
        # Use up through relu4_4 (index 27 in torchvision's VGG-19).
        self.feature_extractor = nn.Sequential(*list(vgg.children())[:28])

    def forward(self, pred: torch.Tensor, target: torch.Tensor) -> torch.Tensor:
        with torch.no_grad():
            t_feats = self.feature_extractor(target)
        p_feats = self.feature_extractor(pred)
        return F.l1_loss(p_feats, t_feats)


def compute_ssim(pred: torch.Tensor, target: torch.Tensor) -> float:
    """SSIM over a (b, 3, H, W) batch; reported as a scalar."""
    from skimage.metrics import structural_similarity as ssim
    import numpy as np

    p = pred.detach().cpu().numpy()
    t = target.detach().cpu().numpy()
    scores = []
    for i in range(p.shape[0]):
        # SSIM per RGB channel, then average.
        per_channel = []
        for c in range(3):
            per_channel.append(ssim(p[i, c], t[i, c], data_range=1.0))
        scores.append(float(np.mean(per_channel)))
    return float(np.mean(scores))


def train(
    data_dir: Path,
    checkpoint_dir: Path,
    epochs: int,
    batch_size: int,
    learning_rate: float,
    device: str,
) -> None:
    logging.basicConfig(
        level=logging.INFO,
        format="%(asctime)s | %(levelname)s | %(message)s",
        handlers=[
            logging.FileHandler(checkpoint_dir / "training.log"),
            logging.StreamHandler(),
        ],
    )
    log = logging.getLogger("train")

    low_dir = data_dir / "720p"
    high_dir = data_dir / "1440p"
    dataset = OracleFrameDataset(low_dir, high_dir)
    n = len(dataset)
    val_n = max(1, n // 10)
    train_n = n - val_n
    train_set, val_set = torch.utils.data.random_split(
        dataset,
        [train_n, val_n],
        generator=torch.Generator().manual_seed(2026),
    )
    train_loader = DataLoader(train_set, batch_size=batch_size, shuffle=True, num_workers=2)
    val_loader = DataLoader(val_set, batch_size=batch_size, num_workers=2)

    model = TemporalUpscalerV1().to(device)
    log.info("TemporalUpscalerV1 parameter count: %d", parameter_count(model))

    optimizer = AdamW(model.parameters(), lr=learning_rate, weight_decay=1e-4)
    scheduler = CosineAnnealingLR(optimizer, T_max=epochs)
    perceptual = PerceptualLoss().to(device)

    best_ssim = 0.0
    for epoch in range(1, epochs + 1):
        # Train.
        model.train()
        train_loss = 0.0
        for history, target in train_loader:
            history = history.to(device)
            target = target.to(device)
            optimizer.zero_grad()
            pred = model(history)
            l1 = F.l1_loss(pred, target)
            perc = perceptual(pred, target)
            loss = l1 + 0.1 * perc
            loss.backward()
            optimizer.step()
            train_loss += loss.item()
        train_loss /= max(1, len(train_loader))

        # Validate.
        model.eval()
        val_ssim = 0.0
        with torch.no_grad():
            for history, target in val_loader:
                history = history.to(device)
                target = target.to(device)
                pred = model(history)
                val_ssim += compute_ssim(pred, target)
        val_ssim /= max(1, len(val_loader))

        scheduler.step()
        log.info(
            "epoch %3d: train_loss=%.4f  val_ssim=%.4f  lr=%.6f",
            epoch,
            train_loss,
            val_ssim,
            scheduler.get_last_lr()[0],
        )

        if val_ssim > best_ssim:
            best_ssim = val_ssim
            best_path = checkpoint_dir / "best.pt"
            torch.save(
                {
                    "epoch": epoch,
                    "model_state_dict": model.state_dict(),
                    "val_ssim": val_ssim,
                },
                best_path,
            )
            log.info("  -> saved best checkpoint (ssim=%.4f)", val_ssim)
        if epoch % 10 == 0:
            ckpt_path = checkpoint_dir / f"epoch_{epoch:03d}.pt"
            torch.save({"epoch": epoch, "model_state_dict": model.state_dict()}, ckpt_path)

    log.info("training complete: best validation SSIM = %.4f", best_ssim)


def main() -> None:
    p = argparse.ArgumentParser()
    p.add_argument("--data-dir", type=Path, default=Path(__file__).parent.parent / "data")
    p.add_argument("--checkpoint-dir", type=Path, default=Path(__file__).parent.parent / "checkpoints")
    p.add_argument("--epochs", type=int, default=100)
    p.add_argument("--batch-size", type=int, default=8)
    p.add_argument("--lr", type=float, default=3e-4)
    p.add_argument(
        "--device",
        type=str,
        default="cuda" if torch.cuda.is_available() else "cpu",
    )
    args = p.parse_args()

    args.checkpoint_dir.mkdir(parents=True, exist_ok=True)
    train(
        args.data_dir,
        args.checkpoint_dir,
        args.epochs,
        args.batch_size,
        args.lr,
        args.device,
    )


if __name__ == "__main__":
    main()
