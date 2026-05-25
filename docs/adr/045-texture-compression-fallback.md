# ADR-045 — Texture compression fallback (BC7/BC5/BC4)

- Status: Accepted (Phase 5 design contract; implementation lands in
  Phase 5 PR 2 / PR 3)
- Date: 2026-05-24
- Phase: 5 — RENDERING FOUNDATION (Track A)
- Companion: ADR-008 (content-addressed asset pipeline), ADR-019
  (asset sandbox subprocesses), ADR-044 (bindless heap)

## Context

Spec §IV.4.A line 404:

> BC7 for albedo/diffuse, BC5 for normals, BC4 for single-channel
> (roughness, AO, metallic). ASTC is the mobile/console path when
> those targets are added.

That fixes the codec choice per channel type. Open question: what
happens when the target hardware does not support the codec? Most
relevant cases:

- **WebGPU on Chrome/Safari, 2026.** BC formats are
  `texture-compression-bc` (Vulkan-class hardware) — supported on
  desktop, optional on mobile. WebGPU also exposes ETC2 and ASTC for
  mobile.
- **Older D3D11-class GPUs.** Rare on the RX 580 milestone target
  (RX 580 is D3D12 / Vulkan 1.3 capable), but the hardware
  compatibility minimum tier (Intel HD 620 / Vega 8) is D3D11
  Feature Level 11_1; BC4/BC5/BC7 are all supported there too.
- **CI runners.** GitHub-hosted runners with software rasterization
  (LLVMpipe) may not advertise BC.

So the realistic fallback question is **"what if BC7 specifically is
missing"** (it's the most modern of the three; BC4/BC5 are older).
ASTC and ETC2 are out of Phase 5 scope (mobile is Phase 11+).

## Decision

### 1. Detection — import time and runtime

**Import time** (`engine-asset-import` future tool, or the existing
asset-pipeline CLI): every texture is compressed to BC{4,5,7} per its
channel role and the compressed bytes are stored in the pak. If the
import-time tool cannot produce the codec (no compressor available
on the build machine), the import fails with a hard error — paks
must contain compressed data.

**Runtime** (`engine-gpu` initialization): query
`wgpu::Features::TEXTURE_COMPRESSION_BC`. If absent (only on web
mobile or pathological CI), the engine refuses to load textures
that require BC and emits a clear error.

The fallback discussion is therefore *not* a runtime substitution —
the asset format is BC, period. If hardware can't sample BC, the
engine reports the hardware as below the spec's minimum tier (spec
Part XX.7). Better than silently de-quality the visuals.

### 2. Per-channel-role codec table

| Channel role | Codec | Block | Notes |
|---|---|---|---|
| Albedo / Diffuse / Emissive | BC7 (sRGB) | 16 B / 4×4 | sRGB-aware |
| Normal | BC5 | 16 B / 4×4 | Two-channel; Z reconstructed in shader |
| Roughness / Metallic / AO (single channel) | BC4 | 8 B / 4×4 | Greyscale |
| Combined Roughness+Metallic+AO | BC7 (linear) | 16 B / 4×4 | When channels packed; linear |
| HDR cube-map (IBL specular) | BC6H_UFLOAT | 16 B / 4×4 | High-dynamic-range |
| UI / pre-multiplied alpha | BC7 (sRGB) | 16 B / 4×4 | RGBA |
| Special: 1×1 fallback magenta | RGBA8 uncompressed | 4 B | Bindless slot 0 |

### 3. Mipmap policy

All compressed textures ship complete mip chains down to 1×1, baked
at import time. The bake uses high-quality filters (Kaiser for
albedo, sobel-based for normal, point for masks). No runtime mip
generation — predictable VRAM and zero per-load latency.

### 4. Asset pak metadata

Each `TextureAsset` in the pak carries:

```rust
struct TextureMeta {
    format: TextureFormat,     // wgpu format enum, mapped to engine-side
    extent: Extent3d,
    mip_count: u8,
    channel_role: ChannelRole, // Albedo|Normal|RoughMetAo|Hdr|Ui
}
```

The `ChannelRole` is the *intent* (what the texture is *for*); the
`format` is the on-disk codec. The two are independent so e.g. a
debug build could ship uncompressed albedo and still hit the right
sampler / shader path.

### 5. Web target — ASTC alternative paks

Phase 5 is native (Linux/Windows). When the web target lands (Phase
9 per spec §IV.4.B + §VI.1 for the Hub web-export), web-only paks
ship parallel asset variants compressed with ASTC LDR 6×6 (albedo)
+ ETC2 (normal). Selected at pak-mount time by the target. This is
out of Phase 5 scope; documented here so the asset-pipeline trait
isn't designed in a way that prevents it.

## Consequences

- Asset import becomes lossy at build time. The reference (lossless)
  source images live in the source tree; only the BC-compressed paks
  ship.
- VRAM cost predictable per texture: 4096×4096 BC7 = 16 MiB (vs.
  64 MiB RGBA8). The bindless heap (ADR-044) sees compressed sizes.
- The compressor tool is a build-time dependency. Permitted compressors
  (under ADR-019 sandbox subprocess pattern):
  - `astcenc` (Arm, BSD-3) — handles ASTC; can also do BC via
    intel-ispc-compatible plugin
  - `nvtt` (NVIDIA Texture Tools) — proprietary; rejected
  - `bcndecode` / `oxipng` — fallback for free formats
  - Phase-5 default: a Rust-native compressor such as `intel_tex`
    (MIT, intel-ISPC-based) used via the sandbox-subprocess
    pattern. Final pinning is the Phase 5 PR 2 implementation
    decision; not pinned in this ADR because the spec already
    constrains the *output*, not the *tool*.
- The decision to refuse-load rather than fall back is a hard line
  but it matches the spec's tier minimums. Below-tier hardware is
  explicitly out of support.

## Risks and tradeoffs

- **BC7 compression is slow at import time** (seconds per 4K
  texture at high quality). Acceptable: import is a build-time cost,
  not a runtime one. The pak is content-addressed, so unchanged
  textures don't re-compress (ADR-008).
- **BC formats are licensed S3TC; the patents expired in 2017.** No
  legal risk in 2026+.
- **Web mobile loses BC support.** Documented; web is Phase 9.
- **CI runners with software rasterization may not advertise BC.**
  CI's render integration tests run against the rasterizer testbed
  (CPU; codec-agnostic) and the engine-shader reproducibility goldens
  (Slang compilation, no GPU). The pixel-parity oracle (ADR-046) is
  the GPU regression test and requires a hardware-capable runner —
  out of CI default, run as a manual / nightly job.
- **Refuse-to-load on hardware below tier is harsh.** Mitigated by
  the editor reporting hardware tier prominently at launch (Phase
  10) and the Hub gating downloads to compatible hardware.

## Alternatives considered

- **Runtime trans-codec (KTX2 + Basis Universal).** Ships ETC1S/
  UASTC universal textures, transcoded to BC/ASTC/ETC at load time.
  Phase 6+ candidate when web mobile lands; deferred because the
  Basis library is large and the native engine doesn't need it.
- **Uncompressed fallback paks.** Doubles pak size. Rejected: the
  hardware tier minimum already supports BC.
- **Per-platform pak variants pre-shipped.** Used by AAA console
  shops. Phase 11+; not now.

## Verification

- Implementation lands with Phase 5 PR 2 (the import tool) and PR 3
  (the runtime texture loader). Tests:
  - `tests/texture_codec_roundtrip.rs`: import a known reference
    image as BC7, BC5, BC4; decode and compare against tolerated
    SSIM ≥ 0.97 (Phase 5's quality bar for albedo / normal /
    single-channel respectively).
  - `tests/texture_format_refusal.rs`: simulate a GPU without BC
    support, assert the loader returns the documented error
    rather than a panic.
- Telemetry: `GAUGE "render.texture_vram_bytes"` per frame;
  `COUNTER "render.texture_load_total"`; `COUNTER
  "render.texture_load_failed"` (the latter should stay at zero).
- No CI guard specific to textures — `wgpu::` boundary (ADR-049)
  catches anything off-boundary.
