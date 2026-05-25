# ADR-005 — Vendor upscaler first, owned fallback

- Status: Accepted
- Date: 2026-05-18 (expanded 2026-05-24 per audit §15 Phase-0 ADR sweep)
- Phase: 0 (pre-v1.0 — contract-exempt per risk R-03)
- Companion: ADR-053 (Phase 5 PR 5 — trait surface realisation),
  ADR-046 (oracle — verifies the owned fallback path), ADR-039
  (render graph — hosts the upscaler pass)

## Context

Modern realtime renderers do not present at the resolution they
render. Temporal upscaling — render at, e.g., 1440p internal,
upscale to 4K display — is the difference between meeting and
missing the frame-pacing milestone (ADR-016 / ADR-047) on the
spec's reference hardware (RX 580 at 1440p, spec §XVIII).

The 2026 upscaling landscape:

- **DLSS 4** (NVIDIA RTX 20-series+). Tensor-core-accelerated;
  best perceptual quality on supported GPUs. Closed source,
  vendor SDK (Streamline 2.x).
- **FSR 4** (AMD RDNA 4 + future Vulkan/D3D12). Cross-vendor as a
  fallback; tensor-accelerated on RDNA 4, spatial on older AMD
  and other vendors. Open source.
- **XeSS 2** (Intel Arc). DP4a-accelerated on cross-vendor
  GPUs; tensor-accelerated on Arc; quality between FSR 4 and
  DLSS 4 on most scenes.
- **Owned ONNX temporal upscaler.** The fully owned fallback;
  Phase 6 spec target (spec line 1634); Phase 5 ships a bilinear
  placeholder so the trait surface is end-to-end testable from
  PR 5 onward.

Three properties matter for the architectural decision:

1. Vendor upscalers exploit hardware the engine cannot replicate
   in software (DLSS's tensor cores, FSR 4's AI accelerators on
   RDNA 4). Where supported, they win on latency and quality.
2. Vendor upscalers are closed/proprietary; the spec's R-02 "own
   the layer" stance considered but rejected per ADR-025's
   reasoning (the engine owns the *use* of the primitive, not
   the primitive itself).
3. Universal coverage requires an owned fallback. RX 580 does not
   meet DLSS hardware requirements; web target (ADR-006) has no
   access to any vendor SDK. The owned upscaler covers both.

## Decision

The renderer exposes an `UpscalerProvider` trait. Four
implementations ship in the engine (Phase 5 PR 5 lands the trait
surface and stubs; Phase 6 expands the vendor bindings and
delivers the owned ONNX path):

```rust
pub trait UpscalerProvider {
    fn name(&self) -> &'static str;
    fn supports(&self, device: &Device) -> bool;
    fn upscale(&self, ctx: &UpscaleCtx) -> Result<UpscaleResult, UpscaleError>;
}
```

Implementations:

- `Vendor::Dlss` — wraps NVIDIA Streamline. `supports()` returns
  true on RTX 20+/40+/50+ with a Streamline-loadable driver.
- `Vendor::Fsr` — wraps AMD FSR 4 SDK. `supports()` returns true
  on RDNA 4 (FSR 4 tensor path) or any DX12/Vulkan device (FSR
  3.x spatial fallback).
- `Vendor::Xess` — wraps Intel XeSS 2 SDK. `supports()` returns
  true on every GPU XeSS recognises (its own internal feature
  detection).
- `Owned::Bilinear` — Phase 5 placeholder. Always supported.
- `Owned::OnnxTemporal` — Phase 6+ deliverable; the full owned
  ONNX temporal upscaler against the rasterizer oracle (ADR-046)
  for verification.

Selection logic (Phase 5 PR 5):

1. Probe registered providers in priority order (vendor > best
   match > owned).
2. The first whose `supports(device)` returns true is selected.
3. The selection is logged via the telemetry channel (ADR-010);
   the user can override via configuration.

## Rationale

The trait surface separates "game code that wants upscaling"
from "the specific upscaler available on this hardware." Game
code calls `ctx.upscale(internal_buffer, jitter)`; the trait
binding handles the rest. Switching upscalers (e.g. a DLSS-to-
FSR fallback when DLSS load fails) is a runtime decision the
trait makes invisible.

Owning the *trait* without owning the *implementations* is the
same pragma as ADR-025 (audited crypto): the engine owns the
contract, the use, and the verification; the vendor owns the
proprietary math. This keeps the engine's binary size bounded
(vendor SDKs are dynamically loaded; not present on systems
that won't use them) and keeps the engine's behaviour
verifiable end-to-end via the owned fallback path on the oracle.

## Consequences

- Phase 5 ships the trait + 4 stubs (3 vendor stubs that return
  "not supported" until Phase 6 bindings land + bilinear owned
  placeholder).
- Phase 6 expands each vendor stub to a real binding; the trait
  surface does not change.
- Phase 6 also delivers the owned ONNX temporal upscaler. The
  bilinear placeholder remains as the "no jitter / no history"
  fallback for legacy paths.
- The oracle (ADR-046) verifies the owned upscaler against a
  golden reference; vendor upscalers are not oracle-verified
  (they are opaque proprietary code), but their *integration*
  is verified by the trait surface tests.
- Configuration surface: `engine.toml` (Phase 6+) gains an
  `[upscaler]` section for override behaviour.

## Risks and tradeoffs

- **Vendor SDK churn.** DLSS Streamline 1→2→3 has involved API
  breaks; FSR's SDK evolves with hardware generations. Mitigation:
  the vendor wrappers are thin; an SDK bump is a contained PR.
- **License surface.** Each vendor SDK has its own license;
  `deny.toml` policies will need per-binding allowances. Tracked
  for Phase 6 work.
- **Owned ONNX dependency.** Phase 6 will introduce an ONNX
  runtime dependency (`ort` or equivalent); per the R-02 stance
  this is acknowledged as an external dep with a vendor binding.
- **The "best match" selection rule is heuristic.** Real-world
  behaviour (e.g. DLSS fails to load due to driver age, fall
  back to FSR which exists but is XeSS-quality on this GPU)
  needs the override mechanism to be available from PR 5
  onward — and visible in telemetry.

## Alternatives considered

- **Owned upscaler only.** Universal portability; loses the
  vendor-hardware advantage on hardware that has it. Rejected.
- **Vendor upscalers only.** Faster on supported hardware;
  forces the RX 580 (no DLSS, no FSR 4 tensor) and the web
  target into "no upscaling, render at full resolution"; misses
  the milestone. Rejected.
- **Hardcode one vendor.** Locks the engine to a single GPU
  ecosystem. Rejected against the spec's portability goal.
- **A `dyn UpscalerProvider` runtime-dispatch trait.** Considered
  but the static enum + `Box<dyn>` registration model wins on
  per-frame cost (one virtual call per frame is not a hot path
  concern). Static dispatch chosen for the small ADR-005 surface.

## Verification

- Phase 5 PR 5: trait surface compiles; bilinear path passes the
  oracle's "upscale fixture" (rendered at 720p, upscaled to
  1440p, compared to a 1440p-rendered reference within an SSIM
  tolerance set by the rasterizer oracle's exception register).
- Phase 5 PR 6: the frame-pacing milestone bench (ADR-047) runs
  with the bilinear placeholder; the milestone gate measures
  the trait-surface-correct end-to-end pipeline even before the
  vendor bindings exist.
- Phase 6: per-vendor binding tests verify each `supports()`
  returns true on the expected hardware and the upscale path
  produces a buffer of the requested dimensions.
- The ONNX temporal upscaler (Phase 6+) has its own oracle suite
  per ADR-046.
