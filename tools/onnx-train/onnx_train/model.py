"""TemporalUpscalerV1 model architecture (ADR-080 §4).

Compact CNN with temporal attention + sub-pixel convolution
upsampling.

References:
    - Goodfellow Ch. 9 — convolutional networks (backbone).
    - Goodfellow Ch. 10 — sequence modeling (temporal attention).
    - Goodfellow Ch. 14 — autoencoders (conceptual framing).
    - Shi et al. 2016 — Real-Time Single Image and Video
      Super-Resolution Using an Efficient Sub-Pixel Convolutional
      Neural Network (the sub-pixel conv upsampling).
"""

from __future__ import annotations

import torch
from torch import nn


class ResidualConvBlock(nn.Module):
    """Two-conv residual block with ReLU activation."""

    def __init__(self, channels: int) -> None:
        super().__init__()
        self.conv1 = nn.Conv2d(channels, channels, kernel_size=3, padding=1)
        self.conv2 = nn.Conv2d(channels, channels, kernel_size=3, padding=1)
        self.relu = nn.ReLU(inplace=True)

    def forward(self, x: torch.Tensor) -> torch.Tensor:
        residual = x
        out = self.relu(self.conv1(x))
        out = self.conv2(out)
        return self.relu(out + residual)


class TemporalAttention(nn.Module):
    """LSTM-style attention over a 4-frame trailing history."""

    def __init__(self, channels: int, history_len: int = 4) -> None:
        super().__init__()
        self.history_len = history_len
        self.hidden_channels = channels
        # Per-frame embedding + per-frame gate. The hidden state is
        # the accumulated temporal representation.
        self.gate = nn.Conv2d(channels * 2, channels, kernel_size=1)
        self.update = nn.Conv2d(channels * 2, channels, kernel_size=1)
        self.sigmoid = nn.Sigmoid()
        self.tanh = nn.Tanh()

    def forward(self, frames: torch.Tensor) -> torch.Tensor:
        # frames: (batch, history_len, channels, H, W).
        # Iterate over history_len with a running hidden state.
        b, t, c, h, w = frames.shape
        assert t == self.history_len, f"expected {self.history_len} frames, got {t}"

        hidden = torch.zeros(b, c, h, w, device=frames.device, dtype=frames.dtype)
        for i in range(t):
            x = frames[:, i]
            combined = torch.cat([x, hidden], dim=1)  # (b, 2c, h, w)
            g = self.sigmoid(self.gate(combined))
            u = self.tanh(self.update(combined))
            hidden = g * hidden + (1 - g) * u
        return hidden


class TemporalUpscalerV1(nn.Module):
    """End-to-end temporal upscaler.

    Input: a sequence of 4 trailing low-res frames (each 3-channel
    RGB) + their motion vectors + depth.
    Output: a single high-resolution RGB frame at 2× spatial scale.
    """

    def __init__(self, internal_channels: int = 64) -> None:
        super().__init__()
        # Per-frame backbone.
        self.input_proj = nn.Conv2d(3, internal_channels, kernel_size=3, padding=1)
        self.backbone = nn.Sequential(
            ResidualConvBlock(internal_channels),
            ResidualConvBlock(internal_channels),
            ResidualConvBlock(internal_channels),
        )
        # Temporal attention over the 4-frame history.
        self.temporal = TemporalAttention(internal_channels, history_len=4)
        # Sub-pixel conv upsampling (Shi et al. 2016): produce 4×3 = 12
        # channels at internal-res, then PixelShuffle(2) → 3 channels
        # at 2× spatial res.
        self.upsample_conv = nn.Conv2d(
            internal_channels, 3 * (2 * 2), kernel_size=3, padding=1
        )
        self.pixel_shuffle = nn.PixelShuffle(upscale_factor=2)

    def forward(self, history: torch.Tensor) -> torch.Tensor:
        # history: (batch, 4, 3, H, W) — 4 trailing low-res frames.
        b, t, _, h, w = history.shape
        # Project + backbone each frame independently (sharing weights).
        embed = history.view(b * t, 3, h, w)
        embed = self.input_proj(embed)
        embed = self.backbone(embed)
        embed = embed.view(b, t, -1, h, w)
        # Temporal accumulator.
        accum = self.temporal(embed)
        # Sub-pixel upsampling.
        up = self.upsample_conv(accum)
        return self.pixel_shuffle(up)


def parameter_count(model: nn.Module) -> int:
    """Return the total number of trainable parameters."""
    return sum(p.numel() for p in model.parameters() if p.requires_grad)


if __name__ == "__main__":
    model = TemporalUpscalerV1()
    n = parameter_count(model)
    print(f"TemporalUpscalerV1 parameter count: {n:,}")
    # Smoke test the forward pass.
    history = torch.randn(1, 4, 3, 720, 1280)
    output = model(history)
    print(f"forward pass: input={tuple(history.shape)}, output={tuple(output.shape)}")
    assert output.shape == (1, 3, 1440, 2560), output.shape
