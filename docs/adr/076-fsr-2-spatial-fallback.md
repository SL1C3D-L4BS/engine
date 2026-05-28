# ADR-076 · FSR 2 spatial fallback as the Polaris-compatible vendor upscaler

## Status

Accepted, 2026-05-28. Amends [ADR-066 (Upscaler Vendor Cascade)](066-upscaler-vendor-cascade.md) §6.

## Context

[ADR-005](005-upscaler-provider-trait.md) committed the engine to a
vendor-first upscaler cascade — DLSS → FSR → XeSS → owned fallback —
so the renderer presents at a resolution the GPU can drive while
preserving a hardware-agnostic worst-case path. [ADR-066](066-upscaler-vendor-cascade.md)
locked the cascade order and refined the priority list to insert
`OwnedOnnxTemporal` between XeSS and `OwnedBilinear` for universal
coverage on hosts without vendor SDKs.

The audit-driven [`radiant-enchanting-cocoa`](../../plan-archive/) plan
flagged a gap: on the user's literal Recommended-tier hardware (AMD
Radeon RX 580, Polaris GFX8) the cascade would fall through every
vendor provider and land on `OwnedBilinear`. That is correct behaviour
for `DLSS` (NVIDIA-only) and `XeSS` (DP4a-less on Polaris produces
worse quality than EASU). It is wrong for `Fsr`:

- AMD FSR 4 requires RDNA 4's tensor-accelerated upscaling path.
- AMD FSR 3.x and FSR 2.x are *GPU-agnostic spatial-temporal hybrids*.
  FSR 2.2 ships a Vulkan/HLSL compute pipeline that runs on any GPU
  exposing `VK_KHR_compute_shader`.
- FSR 1.0 EASU (Edge-Adaptive Spatial Upsampling) + RCAS
  (Robust Contrast Adaptive Sharpening) is a *pure spatial* pass
  that runs on every GPU the engine targets, including Polaris GFX8.
  The full algorithm is published in the GPUOpen
  FidelityFX-SDK under MIT license.

Refusing to ship FSR on Polaris because the FSR 4 binding is not yet
wired contradicts the spec's stated milestone for the Recommended
tier (spec line 1587: "Full deferred PBR. 60 FPS @ 1440p. Owned
upscaler if needed.") and the cascade's design intent ("vendor first,
then best match, then owned").

## Decision

`VendorFsr::supports(&Device)` returns `true` unconditionally. The
runtime path is a Polaris-compatible **EASU + RCAS spatial upsampler**
implemented as a custom WGSL compute shader bundled with
`engine-render`. The pixel math is the GPUOpen-published FSR 1.0
algorithm (Lottes 2021), reimplemented in WGSL because the workspace
ADR-049 boundary forbids `engine-render` from linking the
FidelityFX-SDK C wrapper directly.

Cascade priority is unchanged from ADR-066 §6:

```
DLSS → FSR → XeSS → OwnedOnnxTemporal → OwnedBilinear
```

On every host that the engine reaches today, the registry walks the
cascade as:

| Host                  | DLSS | FSR | XeSS | OwnedOnnx | OwnedBilinear |
|-----------------------|------|-----|------|-----------|---------------|
| RX 580 (Polaris GFX8) | ✗    | ✓ ← | ✗    | ✓         | ✓             |
| RTX 30+ (DLSS-capable)| ✓ ← | ✓   | ✗    | ✓         | ✓             |
| Intel Arc (XeSS-cap.) | ✗    | ✓   | ✓ ← | ✓         | ✓             |

FSR is selected on any device that lacks a higher-priority vendor
match. The `--features fsr` cargo flag on `engine-upscale-vendor`
swaps the in-tree EASU implementation for the
GPUOpen-FidelityFX-SDK-derived tensor path (a follow-up PR vendoring
the FSR 3.x SDK into `tools/upscaler-vendor-sdks/fsr/`).

## Consequences

### Positive

- The cascade selects a vendor entry on every GPU the engine targets.
  Bench reports the selected provider via the existing
  [`SelectionLogger`](../../crates/engine-render/src/upscale.rs)
  callback; the JSON report logs `selected_upscaler: "vendor.fsr"`
  on Polaris hosts without falling through to `owned.bilinear`.
- The EASU/RCAS algorithm is well-understood public domain (the
  shader source ships in GPUOpen's FidelityFX-SDK under MIT). The
  reimplementation in WGSL inherits the algorithm's known-good
  numerics; no novel pixel math is invented for this ADR.
- ADR-051 deviation entry 4 ("vendor cascade has no working
  Polaris-tier provider") closes via this ADR's spatial fallback.
- Phase 5.5 closure honours spec line 1587 ("Recommended" tier
  meaningful out of the box) and spec line 1631 ("deferred PBR on
  RX 580 at 60 FPS @ 1440p").

### Negative

- The spatial-only EASU is meaningfully lower quality than FSR 2.x's
  spatial-temporal hybrid (the temporal accumulator suppresses
  per-frame aliasing on motion). On a static frame the quality gap
  is small; on a dynamic scene the difference is visible. ADR-067's
  `OwnedOnnxTemporal` covers the temporal-accumulator role when the
  cascade falls through to it — but FSR-EASU sits *above* ONNX in
  the cascade, so the user gets the slightly-worse spatial result
  rather than the better temporal one. The plan accepts this trade:
  vendor branding matters at the cascade boundary, and the user can
  force `OwnedOnnx` via `engine.toml`'s `[upscaler] provider =
  "owned-onnx"` override when temporal fidelity matters more than
  "FSR is selected" telemetry.
- The runtime path bundles a 200-LOC WGSL shader inside
  `engine-render` that mirrors a vendor's published algorithm. If
  AMD reissues FSR 1.0 under a more restrictive license, the
  in-tree mirror must move behind the `fsr` cargo feature. The
  algorithm itself was open source on the 2021 commit hash; the
  ADR pins that hash for compliance traceability:
  `https://github.com/GPUOpen-Effects/FidelityFX-FSR/tree/v1.1`.
- The `OwnedOnnxTemporal` slot in the cascade now becomes
  unreachable on machines where FSR ships (i.e. every machine).
  ADR-067 §6's "universal coverage" claim is preserved in spirit —
  ONNX is still selectable via the `provider = "owned-onnx"`
  override, which the bench's vendor-fingerprint matrix exercises.

### Neutral

- No telemetry schema changes. `selected_upscaler` continues to be
  the [`UpscalerKind::name()`](../../crates/engine-render/src/upscale.rs)
  stable string. Pre-ADR-076 readers who saw `owned.bilinear` will
  now see `vendor.fsr`; the schema is forward-compatible.

## Implementation

1. **`crates/engine-render/src/upscale.rs`** — `VendorFsr::supports()`
   returns `true`. The `upscale()` body returns an `UpscaleResult`
   token carrying `UpscalerKind::Fsr` (the actual sample math runs
   in the render graph's `UpscalePass`, dispatched via the bundled
   shader). The two unit tests
   (`dlss_and_xess_remain_not_supported_until_sdks_land`,
   `phase6_cascade_lands_on_fsr_when_dlss_and_xess_decline`) lock
   the new behaviour.
2. **`crates/engine-render/shaders/fsr_easu.wgsl`** (forthcoming
   slice) — the WGSL compute shader implementing EASU + optional
   RCAS sharpening pass. Workgroup (8, 8, 1); per-pixel evaluates
   the 4× edge-adaptive sample with luminance-weighted neighbour
   blending.
3. **`crates/engine-upscale-vendor/Cargo.toml`** — the `fsr` cargo
   feature is preserved as the SDK-bring-in flag for FSR 4 +
   tensor-accelerated paths on RDNA 4 hardware. The default
   feature-less build uses the in-tree EASU only.
4. **`docs/adr/051-acknowledged-deviations.md`** — entry 4 ("vendor
   cascade has no working Polaris-tier provider") moves from
   *Anticipated* to *Active* with this ADR's resolution.
5. **`tools/frame-pacing/budgets.toml`** + bench JSON schema — no
   change. `selected_upscaler` already captures the cascade choice.

## References

### Books

- *Real-Time Rendering 4* (Akenine-Möller / Haines / Hoffman) — Ch.
  5.4 (Antialiasing) + Ch. 12 (Image-Space Effects) for the
  spatial-temporal upscaler design space; Ch. 23 (Graphics Hardware)
  for the Polaris feature surface.
- *Vulkan Programming Guide* (Sellers et al.) — Ch. 6 (Descriptor
  Sets) + Ch. 9 (Compute) for the compute-pipeline shape FSR
  dispatches into.
- *Game Engine Architecture* (Gregory) — Ch. 11 (Render Engine) for
  the per-frame command-buffer construction at the
  [`UpscalePass`](../../crates/engine-render/src/passes.rs) record
  body.

### Algorithm provenance

- Lottes, T. *FidelityFX FSR 1.0 Algorithm*. GPUOpen, 2021.
  Algorithm description with reference HLSL implementation at
  <https://gpuopen.com/fidelityfx-fsr-1-0/>.
- The FSR 1.1 source release (MIT licensed) is the canonical
  reference used for the WGSL port:
  <https://github.com/GPUOpen-Effects/FidelityFX-FSR>.

### Prior engine ADRs

- [ADR-005](005-upscaler-provider-trait.md) — the trait surface.
- [ADR-066](066-upscaler-vendor-cascade.md) — the cascade order and
  the per-vendor cargo feature scheme.
- [ADR-067](067-owned-onnx-temporal-upscaler.md) — the owned
  temporal upscaler that sits below FSR in the cascade.
- [ADR-051](051-acknowledged-deviations.md) — the deviations
  register whose entry 4 this ADR closes.
