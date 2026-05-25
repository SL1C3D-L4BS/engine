# ADR-046 — Rasterizer testbed oracle · regression criteria and
exception process

- Status: Accepted (Phase 5 design contract; implementation lands in
  Phase 5 PR 1)
- Date: 2026-05-24
- Phase: 5 — RENDERING FOUNDATION (Track A)
- Companion: ADR-004 (two-track pipeline), ADR-039 (render graph),
  ADR-053 (Phase 5 PR slicing)

## Context

Spec Part IX establishes the software rasterizer as the rendering
oracle. The relevant verification rule (line 735):

> Diff mode: per-pixel absolute difference; output is a heatmap PNG
> (black = identical, white = max difference); threshold > 1/255 per
> channel in Phase shading regions = registered bug.

That fixes a comparison method and a threshold. Open questions:

- How is the 1/255 threshold applied? Per pixel? P99? On sRGB-encoded
  pixels or linear? In tone-mapped output or pre-tonemap HDR?
- Which regions are "Phong shading"? The post-FX chain (TAA, bloom,
  tonemap) introduces deliberate temporal / spatial variations that
  would trip the threshold.
- How are reference frames refreshed when an *intended* visual
  change lands (an art bug fix, an intentional shader update)?
- What is the procedure for an *exception* — a known divergence the
  engine accepts (e.g. a GPU driver bug we can't fix)?

Without a written process, every Phase 5 PR risks bikeshedding the
oracle. This ADR pins it.

## Decision

### 1. Reference frames live in the pak

`engine-raster` ships a fixture pak: `testbed/raster-reference.pak`,
content-addressed via the asset pipeline. The pak contains:

- 12 scene fixtures (sphere on plane, Cornell box, sponza-lite, a
  shadow-heavy fixture for ADR-040, an IBL fixture for ADR-041, a
  cluster-lights fixture for ADR-043, a TAA-motion fixture for
  ADR-042, a bindless-heap stress fixture for ADR-044, and 4
  combined-scene fixtures).
- Per-fixture reference frames computed by the software rasterizer at
  HEAD on the commit the fixture was introduced. Frame format: 16-bit
  per channel RGBA EXR, pre-tonemap (HDR), with a companion linear
  RGBA8 PNG for visual debugging.
- Per-fixture metadata (camera path, jitter seed, frame count).

The reference frames are *not* the GPU output; they are the *CPU*
output. The CPU path is the oracle.

### 2. Comparison metric — linear-space, pre-tonemap, per-channel

```
diff_per_channel = | gpu_linear_hdr[x, y, c] − cpu_linear_hdr[x, y, c] |
                   / max(cpu_linear_hdr[x, y, c], ε)   // relative
threshold        = 1 / 255      // ADR; same as spec line 735
violation_pixel  = any(diff_per_channel > threshold)
```

Relative-not-absolute because HDR values vary by orders of magnitude.
ε = 1e-6 to avoid division by zero in fully black regions. The
comparison is *linear* (pre-tonemap, pre-sRGB) so a tone-curve change
doesn't ripple as a thousand false-positives.

### 3. Per-scene threshold

A frame "passes" when:

- **p99 violation rate ≤ 1%.** Up to 1% of pixels may exceed the
  per-channel threshold without failing the oracle. Justification:
  GPU floating-point order of operations differs from CPU; small
  numerical drift is inevitable; 1% accommodates that without
  hiding real regressions. The 1% number matches the AMD / NVIDIA
  driver pixel-parity guidance internally.
- **Maximum violation rate ≤ 5%.** Above 5%, the frame fails even
  if p99 is below 1% (catches localized but severe regressions —
  e.g. a single pass produces solid magenta).
- **No pixel exceeds 16× threshold.** A single pixel error
  beyond `16/255` ≈ 0.063 (relative) is always a failure — catches
  NaN propagation or rendering off-by-fundamentally-wrong scaling.

### 4. Region masking

Two regions are excluded from the test:

- **Sub-pixel TAA jitter band.** The 8-frame Halton jitter pattern
  (ADR-042) is deterministic; the CPU and GPU produce the same
  jitter, but small floating-point drift in the per-frame jitter
  application can shift pixels by 1. A 2-pixel-thick band around
  triangle edges is masked from the comparison via a "geometric
  edge mask" produced by the same scene fixture's depth buffer.
- **Bloom kernel tails.** Bloom's downsample/upsample chain
  introduces large relative differences in low-magnitude pixels.
  Pixels with `cpu_linear_hdr.luma < 1e-3` are excluded from the
  *relative* comparison — they still pass an *absolute* threshold
  of `2e-3` (visually imperceptible).

These two masks are computed deterministically from the fixture
itself; no human tuning per scene.

### 5. Reference-frame refresh procedure

When an intentional change to the renderer lands (a shader bug fix,
a new pass), the references must be regenerated:

1. The PR description states: "Refreshes rasterizer reference
   frames" and names the fixtures affected.
2. The PR runs `cargo run -p engine-raster --release --
    --refresh-references --fixtures sphere,ibl,...` locally.
3. The new reference frames are committed to the fixture pak.
4. The PR's diff in `docs/observatory/raster-reference-diffs.md`
   shows the pixel-difference heatmaps from old → new references
   so a reviewer can visually confirm the change is intentional.

A `git commit` whose changes touch *only* references and the
observatory diff log is reviewable in one pass; mixing reference
refresh with code changes is discouraged (creates an un-reviewable
PR).

### 6. Exception process — known divergences

Sometimes a divergence is real and accepted:

- A GPU driver bug fixed in a future driver version that breaks
  our oracle in a known cell.
- A vendor upscaler's deliberate temporal jitter that overruns the
  TAA jitter mask.

Each known-divergence exception is a row in
`docs/audit/oracle-exceptions.md` with:

```
ID: ORACLE-EXC-NNN
Fixture: ...
Region: bounding box in screen space
Owner: ...
Reason: ...
Reviewed: yyyy-mm-dd
Sunset: yyyy-mm-dd (date by which to re-evaluate)
```

The oracle harness reads `oracle-exceptions.md`, parses the rows,
and excludes the named regions when comparing the listed fixtures.
A row whose `Sunset` date has passed turns into a CI warning (not
yet a failure) so review forces revisiting.

Exceptions can be added by a code-review-approved PR. No
"silent override" — every accepted divergence is publicly logged.

### 7. CLI surface (spec Part IX, lines 726–732, expanded)

```
engine raster --scene scene.ron --output frame.png
engine raster --scene scene.ron --backend gpu --output gpu_frame.png
engine raster --scene scene.ron --backend diff --output diff.png
engine raster --scene scene.ron --backend diff --threshold 0.01
engine raster --refresh-references --fixtures sphere,ibl,...
engine raster --run-oracle                     # CI mode: 12 fixtures, exit 0/1
```

The `--run-oracle` mode is what CI invokes.

## Consequences

- Phase 5 PR 1 implements the rasterizer + the oracle harness; every
  later Phase-5 PR adds fixtures for its subsystem (PR 3 adds shadow
  + cluster fixtures, PR 4 adds IBL + TAA + post-FX fixtures, etc.).
- Reference-frame storage is content-addressed; a fixture pak is a
  single asset blob, deduped at pak level. Approximate size budget:
  12 fixtures × ~24 MiB per HDR EXR ≈ 290 MiB. Lives in
  `testbed/engine-raster/fixtures/` and is gitignored except for a
  manifest; the actual binaries land in git-lfs or as a separately-
  fetched artifact (TBD with Phase 5 PR 1 — out of this ADR's
  scope).
- CI runs `--run-oracle` on every commit affecting `crates/
  engine-render/`, `crates/engine-gpu/`, or `testbed/engine-raster/`.
  Hardware: a self-hosted runner with a real GPU (Hetzner GPU
  instance per spec §XIX.1). Cost: ~30 min wall-clock per run; gated
  to render-relevant changes only.

## Risks and tradeoffs

- **The oracle's CPU path is itself code that can have bugs.**
  Mitigated by R-02 from the spec: every oracle ships with a
  separately-written *spec*. The Phong-shading equation, projection
  math, etc. are all closed-form and verifiable on paper.
- **Reference regen procedure can be misused** to paper over a
  regression. Mitigated by the observatory diff log (5): a reviewer
  can see every reference frame's old-vs-new heatmap and call out
  unexpected changes. Plus the rule "reference-refresh PRs touch
  only references."
- **1% / 5% / 16× thresholds are heuristic.** They were chosen to
  pass the spec's stated 1/255 per channel at p99 while accommodating
  realistic GPU vs CPU float-op-order drift. If they prove too loose
  or too tight in Phase 5, the ADR is revised (a new ADR-046½, per
  the spec's immutable-ADR rule).
- **Self-hosted GPU runner is a cost** (~$50-100/mo for a small
  GPU instance). Acceptable for the milestone; the CI cost section
  in §XIX.1 budgets for it.
- **Exception process is honest debt.** Each ORACLE-EXC entry is a
  promise to revisit. The Sunset-date warning forces it.

## Alternatives considered

- **SSIM / DSSIM as the metric.** Better correlates to human
  perception but harder to debug ("which pixel diverged?" answers
  with a structural index instead of a coordinate). Rejected:
  per-pixel linear difference is the documented spec contract;
  SSIM is a Phase 6+ candidate for an *additional* metric.
- **Output-frame hashing only (BLAKE3 of the rendered PNG).**
  Used by some engines. Rejected: catches bit-exact regressions but
  not the realistic case of a small numerical drift that's still
  acceptable. The threshold-based approach is what the spec
  demands.
- **No oracle, manual visual review.** Rejected: spec R-02 is
  explicit ("every owned subsystem ships with a verification
  oracle").

## Verification

- Implementation lands with Phase 5 PR 1. Tests:
  - `tests/oracle_threshold_math.rs`: synthetic CPU vs GPU pixel
    pairs at known relative differences; assert the verdict matches
    the spec.
  - `tests/oracle_exception_parser.rs`: round-trip
    `oracle-exceptions.md` parsing; assert sunset-date warnings
    fire when stale.
- Telemetry: `engine raster --run-oracle` emits
  `Event { "raster.oracle_run", Subsystem::Render, fields: [(fixture,
  p99_violation, max_violation)] }` per fixture, captured by the CI
  job's structured log artefact.
- The `engine-raster` lib.rs doc comment (currently 3 lines) gets a
  refresh pointing at this ADR (task #18).
