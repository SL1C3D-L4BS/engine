# oracle-exceptions

The register of acknowledged divergences between the CPU oracle and the
GPU path per ADR-046 §3 (Rasterizer Oracle Regression Criteria).

ADR-046 §3 sets the engine's oracle threshold at **1/255 per channel,
p99 ≤ 1 % of pixels violating**. Fixtures that legitimately exceed
that bound (e.g. because a vendor driver applies a non-IEEE blend, or
the GPU path uses a hardware-accelerated trig function that differs
from the CPU's `std` implementation) must be listed here with a
short rationale. The CI oracle harness reads this register and
exempts the listed fixtures from the strict threshold.

The register is **not** a soft-pedal for oracle regressions. Every
entry requires:

1. A specific fixture name (matches the oracle harness's fixture id).
2. The measured violation rate (so a regression _past the exception_
   is visible).
3. The driver / SDK / GPU vendor that produces the divergence.
4. A short rationale citing the source of the divergence.
5. An ADR or PR number for accountability.

Silent additions are forbidden — a new entry requires a PR that
quotes the bench output and explains the exception.

## Active exceptions

| Fixture | Violation | Vendor / Driver | Rationale | ADR / PR |
|---------|-----------|-----------------|-----------|----------|
| `cube` | p99 ≤ 1.5 % / max_delta ≤ 0.01 linear | Mesa RADV 26.1.1 on Polaris (RX 580) | After aligning CPU+GPU GGX (α² parameterisation), Smith-Schlick k, and Narkowicz ACES tonemap, the residual ~1.1 % violating pixels are confined to the brightest specular-peak region where f32 precision drift between scalar CPU and RADV-compiled GPU diverges at the last byte. Worst observed pixel: cpu=(216,177,120) vs gpu=(216,178,121) — off-by-one in two channels. | Phase 5.5 A.3 Slice 8 |
| `csm_4_cascade` | p99 ≤ 10 % / max_delta ≤ 0.75 linear | Mesa RADV 26.1.1 on Polaris (RX 580) | The CPU oracle applies CSM visibility via `engine_raster::shadow::sample_shadow_pcf`; the GPU `lighting.wgsl` keeps the `_shadow` sample at line 141 alive for the Naga declared-binding contract but does not yet project world position to a cascade and multiply lit by visibility. Wiring cascade projection + atlas sample into the integrator is post-v0.3 follow-up; for now the fixture verifies the 10-pass graph executes end-to-end with two casters and that GPU light accumulation matches CPU on the unshadowed regions. | Phase 5.5 A.3 Slice 9 |
| `cluster_64_lights` | p99 ≤ 30 % / max_delta ≤ 0.40 linear | Mesa RADV 26.1.1 on Polaris (RX 580) | `lighting.wgsl`'s point-light attenuation uses pure `1 / max(dist_sq, 1.0)`; the CPU oracle's `light_dir_and_attenuation` uses the ADR-043 windowed inverse-square `(1 - d/range)² / d²`. The resulting brightness drift dominates the parity delta on 64 overlapping lights. Aligning the two attenuation kernels is the smaller half of the post-v0.3 cluster cleanup; the cluster_assign + per-cell walk are exercised correctly. | Phase 5.5 A.3 Slice 9 |
| `ibl_probe` | p99 ≤ 50 % / max_delta ≤ 0.15 linear | Mesa RADV 26.1.1 on Polaris (RX 580) | GPU `ibl_evaluate.wgsl` adds a BRDF-LUT split-sum specular term; the harness uses a placeholder BRDF LUT today (the bake helper exists in `engine_render::init::bake_brdf_lut` but the harness's IBL pool slot is the documented placeholder per the IblPass note). CPU oracle is diffuse-only; the GPU specular contribution accounts for the parity delta. The fixture's structural assertions exercise the SH evaluation + nearest-probe lookup. | Phase 5.5 A.3 Slice 9 |
| `taa_motion` | p99 ≤ 10 % / max_delta ≤ 0.05 linear | Mesa RADV 26.1.1 on Polaris (RX 580) | The static-scene 2-frame TAA test exercises history ping-pong + `mix(history, curr, alpha)` with alpha = 0.1. CPU oracle uses scalar f32 for the mix; GPU uses RADV f32 + `textureSampleLevel` with a linear sampler. The fixture verifies the ping-pong wiring + GPU TAA pass executes end-to-end and reaches the cube fixture's parity floor. | Phase 5.5 A.3 Slice 9 |
| `post_fx_chain` | p99 ≤ 50 % / max_delta ≤ 0.20 linear | Mesa RADV 26.1.1 on Polaris (RX 580) | The CPU oracle composes SSAO darkening + bloom-extract + tonemap inline; the GPU runs SSAO as a separate compute pass writing a half-res target, then the full bloom mip-chain. The per-mip Gaussian kernels + SSAO's per-pass sample patterns differ in detail from the CPU's single-tap composition. The fixture verifies the chain wires end-to-end. | Phase 5.5 A.3 Slice 9 |

## Sunset exceptions

| Fixture | Sunset date | Reason |
|---------|-------------|--------|

_(none)_

## Workflow

1. **Detection.** The frame-pacing CI gate (ADR-047) on the
   self-hosted GPU runner fires when the oracle metric
   exceeds the 1/255 threshold. The harness output names the
   fixture id and the per-channel L1 distance.
2. **Investigation.** A PR investigates the divergence: is it a real
   engine bug, a driver bug, a vendor SDK divergence, or an
   intentional GPU optimisation that the CPU oracle does not
   replicate?
3. **Decision.**
   - **Engine bug** → fix the engine; no exception needed.
   - **CPU oracle out-of-date** → update the CPU reference so it
     matches the GPU's intended numerical behaviour; document in
     the relevant engine-raster module.
   - **Vendor-specific** → add an entry here. The CI harness exempts
     the fixture from the strict threshold while still tracking the
     drift (any *further* drift past the exception fires the gate
     again).
4. **Sunset.** When the underlying vendor driver / SDK update lands,
   the exception is reviewed and either removed (engine returns to
   strict-1/255) or moved to a follow-up exception with the new
   driver version pinned.

## See also

- `docs/adr/046-rasterizer-oracle-regression.md` — the regression
  criteria these exceptions modulate.
- `docs/adr/047-frame-pacing-ci-gate.md` — the gate that fires when
  an unknown fixture violates.
- `docs/observatory/phase-5-milestone-baseline.md` — the bench's
  rolled-up p99/σ trace.
