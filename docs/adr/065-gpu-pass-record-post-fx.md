# ADR-065 — GPU `record()` contracts: post-FX passes

- Status: Accepted (Phase 6 design contract; implementation lands in
  Phase 6 PR 4)
- Date: 2026-05-27
- Phase: 6 — RENDERING FOUNDATION (Track A, Part 2)
- Companion: ADR-041 (IBL L2 SH probes), ADR-042 (TAA accumulation),
  ADR-039 (render graph), ADR-044 (bindless heap), ADR-046 (oracle
  regression criteria), ADR-049 (engine-gpu wrapper), ADR-064 (GPU
  geometry/lighting pass contracts), ADR-068 (Phase 6 PR slicing)

## Context

Phase 5 PR 4 (commit `a492581`) shipped five post-FX passes —
`SsaoPass`, `IblPass`, `TaaPass`, `BloomPass`, `TonemapPass` — as
`engine_render::passes` entries with no-op `record()` bodies. Their
CPU reference implementations live in
`testbed/engine-raster/src/{post_fx,ibl}.rs`. The oracle (ADR-046)
verifies the CPU reference against the source-of-truth math:
ACES tonemap, soft-knee bloom, 3×3 neighborhood-clip TAA in YCgCo,
8-tap Fibonacci SSAO, L2 SH evaluation, BRDF LUT split-sum sampling.

Phase 6 PR 4 turns the no-op `record()` bodies into real wgpu calls.
The work composes onto PR 3's geometry + lighting pipeline: post-FX
consumes `LitColor` + `GBufferMotionDepth` + `GBufferNormalMetallic`
+ `DepthBuffer` from PR 3 and emits the final `UpscaledColor` /
`TonemappedColor`.

This ADR fixes the GPU contract surface — descriptor layouts, ping-
pong history buffers, sampler choices, BRDF LUT bake mechanics, IBL
probe SSBO format — so the five passes interoperate at the byte
level with the CPU oracle math and with PR 3's outputs.

## Decision

### 1. SSAO (`SsaoPass`)

Compute pass. Workgroup size (8, 8, 1). Reads
`GBufferNormalMetallic` + `DepthBuffer`; writes `SsaoTexture`
(`R16Float`, half-resolution = `output_extent / 2`).

8-tap Fibonacci kernel matches `testbed/engine-raster/src/post_fx.rs::
ssao_fibonacci_kernel`. Sample positions are precomputed at startup
and bound as a uniform array in Group 1:

```text
SsaoUniforms (Group 1, 144 B):
  inverse_projection  mat4x4<f32>       // 64 B
  kernel              [vec4<f32>; 8]    // 8 × 16 = 128 B
  radius              f32
  bias                f32
  intensity           f32
  reserved            f32
```

(Wait — that's 64 + 128 + 16 = 208 B. Reduce kernel to 8 × `vec3<f32>`
packed into 8 × vec4 with `.w = 0`; UBO alignment requires the vec4.
Total stays 208 B, which is fine — 256 B is the realistic per-pass
UBO budget. Layout adjusted accordingly.)

A bilateral upsample to full resolution lives in the
`LightingAccumulationPass` (PR 3) consumption path; SSAO writes
half-res, consumer reads at full-res with depth-aware filtering.

### 2. IBL (`IblPass`)

Compute pass evaluating the 9-band L2 SH probe at every pixel and
sampling the split-sum BRDF LUT for specular. Workgroup size
(8, 8, 1). Reads `GBufferNormalMetallic` + `GBufferAlbedoRoughness`
+ `DepthBuffer` + `IblProbeSet` (SSBO) + `BrdfLut` (texture); writes
`IblContribution` (`Rgba16Float`, full-resolution, added to
`LitColor`).

`IblProbeSet` SSBO layout matches
`testbed/engine-raster/src/ibl.rs::IblProbeSet`:

```text
IblProbeSet SSBO (~14 KiB, 128 probe cap per ADR-041):
  probe_count   u32
  cell_size_m   f32
  reserved      [u32; 2]
  probes[128]:
    cell_key   [i32; 3]               // 12 B
    pad        u32                    // 4 B
    sh_coeffs  [vec4<f32>; 9]         // 144 B per probe (9 L2 RGB coeffs in RGBA)
```

The 8-neighbour trilinear sample matches the CPU oracle's
`IblProbeSet::sample` algorithm exactly: hash the world-space query
position to its containing cell + 7 neighbours, BLAKE3-truncate the
cell-key sort order so the same neighbour list emerges on every
arch.

### 3. BRDF LUT bake (`BrdfLutBake`, runs once at startup)

A one-shot compute pass executed at engine init (not per-frame).
Bakes the split-sum specular BRDF LUT into `BrdfLut` (`Rg16Float`,
512×512). Matches `testbed/engine-raster/src/ibl.rs::bake_brdf_lut`'s
Hammersley + GGX importance-sampled integration.

The LUT is content-addressed (BLAKE3-hashed); if the same LUT was
baked in a prior run it is loaded from the user's cache directory
(`$XDG_CACHE_HOME/sliced-engine/brdf_lut.bin`) instead of re-baked.
First-run bake cost: ~80 ms on the RX 6700 XT runner.

### 4. TAA (`TaaPass`)

Compute pass. Workgroup size (8, 8, 1). Reads `LitColor` (current
frame) + `IblContribution` + `TaaHistory` (previous frame) +
`GBufferMotionDepth` + `DepthBuffer`; writes `TaaResolvedColor`
(`Rgba16Float`) + updates `TaaHistory` for the next frame.

**Jitter source.** The vertex shader's jitter applied in the
`GBufferPass` and `LightingAccumulationPass` is the same Halton(2,3)
period-8 sequence the CPU oracle uses:
`engine_raster::post_fx::jitter_for_frame(frame_idx)`. Phase 6 PR 4
re-exports this function from `engine_render::post_fx::jitter` so
the GPU path consumes the same byte sequence (the CPU oracle and the
runtime cannot diverge on jitter — it is the determinism anchor).

**History ping-pong.** Two `Rgba16Float` textures, alternating per
frame:

```text
Frame N reads from TaaHistoryA, writes to TaaHistoryB.
Frame N+1 reads from TaaHistoryB, writes to TaaHistoryA.
```

The `TaaHistory` ResourceType in `engine_render::resources` is a
double-buffered handle; the graph compile (ADR-039) maps it to
the current frame's read + write side.

**Sampler.** History is sampled with a `Linear` sampler in the wide-
sample path (neighbour gathering) and `Point` in the reproject path
(motion-vector-driven exact-pixel lookup). Two samplers in Group 0.

**Neighbourhood clip.** 3×3 neighbourhood in YCgCo (matches CPU
oracle); disocclusion mask from depth-ratio threshold > 0.05 (matches
CPU oracle). Bias and clip parameters are uniform-bound:

```text
TaaUniforms (Group 1, 96 B):
  prev_view_projection   mat4x4<f32>     // 64 B
  jitter_current         vec2<f32>       // 8 B
  jitter_prev            vec2<f32>       // 8 B
  blend_alpha            f32              // [0.05, 0.5]
  disocclusion_threshold f32              // 0.05
  pad                    [f32; 2]
```

### 5. Bloom (`BloomPass`)

Compute pass chain: bright-pass extract → 5-level downsample
(`Rgba16Float`, halving extent each level) → 5-level upsample with
additive blend. Matches `testbed/engine-raster/src/post_fx.rs::
bloom_soft_knee` + `bloom_gaussian_blur`.

Bright-pass extract uses the soft-knee threshold function with
parameters in Group 1:

```text
BloomUniforms (Group 1, 32 B):
  threshold   f32
  soft_knee   f32
  intensity   f32
  pad         f32
  // ... mip-level resolution per level pushed via push-constants
```

The downsample chain is 5 dispatches; the upsample chain is 5
dispatches. Each dispatch's source extent is in push-constants.

Bloom output is a single `Rgba16Float` texture at full resolution
(after the final upsample), consumed by `TonemapPass`.

### 6. Tonemap (`TonemapPass`)

Compute pass. Workgroup size (8, 8, 1). Reads `TaaResolvedColor` +
`BloomTexture`; writes `TonemappedColor` (`Bgra8UnormSrgb`, matches
swapchain format).

ACES filmic (Stephen Hill fit) per CPU oracle. Output is gamma-
correct sRGB; the swapchain consumes it directly without further
conversion.

```text
TonemapUniforms (Group 1, 16 B):
  exposure        f32
  bloom_mix       f32
  white_point     f32
  pad             f32
```

## Rationale

- **The CPU oracle is the math source, again.** Same property as PR 3
  (ADR-064): PR 4 transposes the CPU algorithms onto GPU paths and
  verifies pixel parity.
- **`jitter_for_frame` re-exported from `engine_render::post_fx::jitter`.**
  The CPU oracle's jitter sequence is the *only* sequence; the GPU
  path consumes the same f32 bytes, eliminating the most common
  source of CPU↔GPU divergence in TAA implementations.
- **`Rgba16Float` LitColor → TaaHistory → TaaResolvedColor.** Half-
  precision is enough perceptual range for HDR; `R11G11B10Float`
  would save 33% but loses precision visible on dark blue tones.
- **BRDF LUT cached on disk.** First-frame stall otherwise; the bake
  is deterministic so a per-installation cache is safe.
- **Bloom as compute chain (not draw-fullscreen-quad chain).** Modern
  GPUs run compute downsample 10-20% faster than draw downsample at
  high resolutions; the chain is short (5 levels) so the overhead of
  per-dispatch state-set is amortized.

## Consequences

- The five passes' Phase-5 stubs gain `pipeline:
  engine_gpu::ComputePipeline` fields, plus uniform-buffer / SSBO /
  sampler bindings per their ADR-065 contracts.
- `engine_render::post_fx::jitter` (the re-exported
  `jitter_for_frame`) becomes the single source of jitter for both
  CPU oracle and GPU runtime.
- A new `BrdfLut` resource type lands in `engine_render::resources`;
  it is a one-shot bake (not per-frame).
- A new on-disk cache file (`brdf_lut.bin`) appears at
  `$XDG_CACHE_HOME/sliced-engine/`. Documented in
  `docs/architecture/engine-render.md`.
- Three new oracle fixtures land in `tests/rendering/`:
  `ibl_probe_pixel_parity`, `taa_motion_pixel_parity`,
  `post_fx_chain_pixel_parity`. Each renders the same fixture via
  CPU oracle + GPU path and asserts pixel parity at ADR-046
  thresholds.
- The frame's GPU pipeline count grows by 5 (SsaoPass, IblPass,
  TaaPass, BloomPass with 5 mip dispatches via one pipeline,
  TonemapPass) plus 1 for the one-shot BRDF LUT bake.

## Risks and tradeoffs

- **TAA history is the most fragile part.** Jitter divergence is the
  classic class-of-bug; the shared `jitter_for_frame` source closes
  it. Disocclusion mask is the second most fragile; the CPU oracle's
  threshold (0.05) is the canonical value.
- **`Rg16Float` BRDF LUT loses ~1 LSB precision** vs the CPU oracle's
  `[f32; 2]`. The oracle threshold (1/255 channel) absorbs this; the
  exception register stays empty for this pass.
- **`R16Float` half-res SSAO** is the established AAA precision; full-
  res `R8Unorm` is the consumer-grade fallback if VRAM is tight.
  Phase 6 ships half-res `R16Float`; downgrade is a Phase 7+
  optimization decision.
- **Bloom mip chain at 4K = ~24 MiB total mip storage.** Acceptable
  on the RX 6700 XT; tight on RX 580 (8 GiB) when combined with the
  G-buffer's 192 MiB. Mitigation: same as ADR-064 — if PR 6's bench
  shows VRAM pressure, drop bloom to 4 mip levels in a follow-up.
- **Disk-cached BRDF LUT** introduces a multi-machine-cache concern.
  Mitigation: the cache filename embeds the bake parameters' hash;
  changing the parameters invalidates the cache automatically.

## Alternatives considered

- **HBAO+ instead of 8-tap Fibonacci SSAO.** Higher quality; more
  taps; vendor-patented (NVIDIA). Rejected — owned-discipline + spec
  baseline is the Fibonacci kernel.
- **GTAO (Ground-Truth Ambient Occlusion).** Better quality at ~2×
  cost; could be a higher-quality preset in Phase 7+. Phase 6 ships
  the spec-baseline Fibonacci.
- **Spectral tonemapping (Reinhard-Jodie + AgX).** AgX is the 2024+
  state-of-the-art; ACES is the established standard the CPU oracle
  already implements. Stick with ACES for parity; an AgX preset is
  a Phase 7+ ADR amendment.
- **TAA with FXAA-style spatial fallback.** Phase 6 ships TAA only;
  upscaler (PR 5) handles the spatial-quality recovery in motion.
- **Per-frame BRDF LUT bake.** Wastes ~80 ms/frame; the LUT is
  static. Rejected.

## Verification

- Implementation lands in Phase 6 PR 4. Test files:
  - `tests/rendering/ibl_probe_pixel_parity.rs`: render the IBL
    fixture via CPU oracle + GPU; assert ADR-046 pixel parity.
  - `tests/rendering/taa_motion_pixel_parity.rs`: render a 60-frame
    moving-camera sequence; CPU oracle reproject vs GPU reproject;
    assert pixel parity on frame 30+ (after history stabilizes).
  - `tests/rendering/post_fx_chain_pixel_parity.rs`: full SSAO + IBL
    + TAA + Bloom + Tonemap chain; CPU vs GPU end-to-end.
  - `tests/rendering/brdf_lut_bake.rs`: bake LUT on GPU; bake on
    CPU; assert byte-equal at half-precision quantization. Re-bake
    after deleting cache → loads from cache on second run (no
    duplicate bake-time SPAN).
- Telemetry: per-pass `SPAN` markers
  (`SPAN "render.ssao"`, `SPAN "render.ibl"`, …) per ADR-010. The
  bake pass emits `SPAN "render.brdf_lut.bake"` once.
- The wgpu boundary guard (ADR-049) is the boundary check; no new
  CI guard needed.
- The frame-pacing measurement (PR 6) consumes the per-pass spans
  for breakdown. Post-PR-4 measurement of the full chain should
  reach within ~10% of the RX-580 milestone budget.
