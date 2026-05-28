# oracle-exceptions

The register of acknowledged divergences between the CPU oracle and the
GPU path per ADR-046 §3 (Rasterizer Oracle Regression Criteria) +
ADR-046 §6a (category scheme amendment, ADR-081).

ADR-046 §3 sets the engine's oracle threshold at **1/255 per channel,
p99 ≤ 1 % of pixels violating**. Fixtures that legitimately exceed
that bound — driver-level vendor drift, architectural CPU↔GPU design
intent splits — are listed below with a short rationale. The CI
oracle harness reads this register and exempts the listed fixtures
from the strict threshold while still tracking per-entry drift (any
*further* drift past the exception fires the gate again).

The register is **not** a soft-pedal for oracle regressions. Every
entry requires:

1. A specific fixture name (matches the oracle harness's fixture id).
2. A *category* (per ADR-046 §6a): `engine-fix`, `cpu-oracle-stale`,
   `vendor-driver`, or `architectural`.
3. The measured violation rate (so a regression *past the exception*
   is visible).
4. The driver / SDK / GPU vendor that produces the divergence (for
   `vendor-driver`) or the design-intent split (for `architectural`).
5. A short rationale citing the source of the divergence.
6. An ADR or PR number for accountability.

Silent additions are forbidden — a new entry requires a PR that
quotes the bench output and explains the exception.

## Active exceptions

| Fixture | Category | Violation | Vendor / Driver | Rationale | ADR / PR |
|---------|----------|-----------|-----------------|-----------|----------|
| `cube` | vendor-driver | p99 ≤ 1.5 % / max_delta ≤ 0.01 linear | Mesa RADV 26.1.1 on Polaris (RX 580) | After aligning CPU+GPU GGX (α² parameterisation), Smith-Schlick k, and Narkowicz ACES tonemap, the residual ~1.1 % violating pixels are confined to the brightest specular-peak region where f32 precision drift between scalar CPU and RADV-compiled GPU diverges at the last byte. Worst observed pixel: cpu=(216,177,120) vs gpu=(216,178,121) — off-by-one in two channels. | Phase 5.5 A.3 Slice 8 |
| `csm_4_cascade` | engine-fix | p99 ≤ 10 % / max_delta ≤ 0.75 linear | Mesa RADV 26.1.1 on Polaris (RX 580) | The CPU oracle applies CSM visibility via `engine_raster::shadow::sample_shadow_pcf`; the GPU `lighting.wgsl` keeps the `_shadow` sample at line 141 alive for the Naga declared-binding contract but does not yet project world position to a cascade and multiply lit by visibility. Wiring cascade projection + atlas sample into the integrator is post-v0.3 follow-up; for now the fixture verifies the 10-pass graph executes end-to-end with two casters and that GPU light accumulation matches CPU on the unshadowed regions. | Phase 5.5 A.3 Slice 9 |
| `ibl_probe` | engine-fix | p99 ≤ 50 % / max_delta ≤ 0.15 linear | Mesa RADV 26.1.1 on Polaris (RX 580) | GPU `ibl_evaluate.wgsl` adds a BRDF-LUT split-sum specular term; the harness uses a placeholder BRDF LUT today (the bake helper exists in `engine_render::init::bake_brdf_lut` but the harness's IBL pool slot is the documented placeholder per the IblPass note). CPU oracle is diffuse-only; the GPU specular contribution accounts for the parity delta. The fixture's structural assertions exercise the SH evaluation + nearest-probe lookup. | Phase 5.5 A.3 Slice 9 |
| `taa_motion` | vendor-driver | p99 ≤ 10 % / max_delta ≤ 0.05 linear | Mesa RADV 26.1.1 on Polaris (RX 580) | The static-scene 2-frame TAA test exercises history ping-pong + `mix(history, curr, alpha)` with alpha = 0.1. CPU oracle uses scalar f32 for the mix; GPU uses RADV f32 + `textureSampleLevel` with a linear sampler. The fixture verifies the ping-pong wiring + GPU TAA pass executes end-to-end and reaches the cube fixture's parity floor — i.e. inherits the cube driver-level exemption rather than introducing a separate divergence. | Phase 5.5 A.3 Slice 9 |
| `post_fx_chain` | architectural | p99 ≤ 50 % / max_delta ≤ 0.20 linear | n/a (architectural design-intent split) | The CPU oracle composes SSAO darkening + bloom-extract + tonemap inline at full resolution; the GPU runs SSAO as a separate compute pass writing a half-res target, then a 5-mip Gaussian-kernel bloom pyramid. The two paths are *not the same algorithm* — they are different design points trading off reference-clarity vs. production-perf. The fixture's structural assertions verify the chain executes end-to-end (every pass runs; output is non-zero; histogram is in the expected range). Per-pixel parity at 1/255 is structurally outside the oracle's design intent. **Permanent exception** per ADR-046 §6a / ADR-081. | ADR-081 |

## Sunset exceptions

| Fixture | Sunset date | Reason | Sunset PR |
|---------|-------------|--------|-----------|
| `cluster_64_lights` | 2026-05-28 | `lighting.wgsl`'s point-light attenuation now matches the ADR-043 windowed inverse-square `(1 - clamp(d/range, 0, 1))² / d²` (the CPU oracle's `engine_raster::shading::light_dir_and_attenuation` formula). The 64-overlapping-lights fixture reaches strict 1/255. | Phase 6 PR 1a (ADR-081) |

## Workflow

1. **Detection.** The frame-pacing CI gate (ADR-047) on the
   self-hosted GPU runner fires when the oracle metric exceeds the
   1/255 threshold. The harness output names the fixture id and the
   per-channel L1 distance.
2. **Investigation.** A PR investigates the divergence: is it a real
   engine bug, a driver bug, a vendor SDK divergence, an intentional
   GPU optimisation that the CPU oracle does not replicate, or an
   architectural design-intent split between reference and production
   paths?
3. **Decision** (per ADR-046 §6a categories):
   - **Engine bug** (category `engine-fix`) → fix the engine; no
     exception needed (or land the entry with a Sunset PR pointing at
     the fix).
   - **CPU oracle out-of-date** (category `cpu-oracle-stale`) →
     update the CPU reference so it matches the GPU's intended
     numerical behaviour; document in the relevant engine-raster
     module; sunset the entry.
   - **Vendor-specific** (category `vendor-driver`) → add an entry
     here. The CI harness exempts the fixture from the strict
     threshold while still tracking the drift (any *further* drift
     past the exception fires the gate again).
   - **Architectural divergence** (category `architectural`) → add
     an entry under category *architectural*; document the design
     intent split (which path is reference, which is production);
     the entry is *permanent* and never sunsets unless one of the
     two paths is redesigned to match the other.
4. **Sunset.** When the underlying vendor driver / SDK update lands
   *or* an engine-fix entry's PR merges, the exception is reviewed
   and either removed (engine returns to strict-1/255) or moved to
   a follow-up exception with the new driver version pinned.
   *Architectural* entries never sunset by this workflow.

## See also

- `docs/adr/046-rasterizer-oracle-regression.md` — the regression
  criteria these exceptions modulate. §6a amendment (ADR-081)
  defines the category scheme.
- `docs/adr/047-frame-pacing-ci-gate.md` — the gate that fires when
  an unknown fixture violates.
- `docs/adr/081-oracle-exception-sunset-and-046-amendment.md` —
  the ADR that introduces the four-category scheme.
- `docs/observatory/phase-5-milestone-baseline.md` — the bench's
  rolled-up p99/σ trace.
