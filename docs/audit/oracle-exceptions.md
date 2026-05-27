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

_(no active exceptions as of 2026-05-26 — the GPU runner is not yet
provisioned)_

## Sunset exceptions

| Fixture | Sunset date | Reason |
|---------|-------------|--------|

_(none)_

## Workflow

1. **Detection.** The frame-pacing CI gate (ADR-047) on the
   self-hosted RX 6700 XT runner fires when the oracle metric
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
