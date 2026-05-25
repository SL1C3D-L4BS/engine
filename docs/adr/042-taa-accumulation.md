# ADR-042 — TAA accumulation and rejection strategy

- Status: Accepted (Phase 5 design contract; implementation lands in
  Phase 5 PR 4)
- Date: 2026-05-24
- Phase: 5 — RENDERING FOUNDATION (Track A)
- Companion: ADR-039 (render graph), ADR-040 (CSM jitter
  coordination), ADR-044 (bindless heap — history buffer)

## Context

Spec §IV.4.A line 384 declares TAA in the post-FX chain:
`SSAO → TAA → Bloom → Tonemap → optional CA/Vignette/Grain →
Upscale`. The spec fixes the position in the chain but not the
algorithm: jitter sequence, history rejection, blend rate, and the
ghosting countermeasures are all open.

TAA is the highest-risk post-effect because it interacts with the
camera projection, the shadow jitter (ADR-040), motion vectors
(emitted by `draw.opaque` per spec §IV.4.A line 379), and the
upscaler input contract (ADR-005 + Phase 5 PR 5). Get any of those
wrong and the output ghosts, smears, or flickers.

The Karis 2014 / Iglesias-Brevik 2020 baseline (neighbourhood-clip,
8-frame Halton, motion-vector reprojection) is the documented
reference in *Real-Time Rendering* 4th ch. 5. That's what this ADR
adopts, with two engine-specific contracts: the jitter sequence is
queryable so the shadow pass (ADR-040) can match it, and TAA's
output is the canonical input to the UpscalerProvider trait
(ADR-005).

## Decision

### 1. Jitter sequence — Halton (2,3), period 8

```
jitter(frame) = (halton(frame, 2) − 0.5, halton(frame, 3) − 0.5) · pixel_size
period = 8 frames
```

The jitter is applied as a sub-pixel offset to the camera projection
matrix. After 8 frames the sample pattern covers the pixel
quasi-uniformly. Period 8 (not 16 or 32) is chosen because:

- Lower periods (4) re-converge faster after camera motion but leave
  visible patterns on static frames.
- Higher periods (16, 32) over-smooth; ghosting countermeasures have
  to work harder.
- 8 is the *RTR4* 4th ch. 5 recommendation and the Frostbite /
  Unreal default.

The jitter is queryable via a public API:
`engine_render::taa::jitter_for_frame(frame: u64) -> Vec2`. ADR-040
consumes it.

### 2. Accumulation — exponential blend, α = 0.05 to 0.5

Per-pixel blend rate:

```
α = lerp(0.05, 0.5, t)    // 0.05 = 20-frame history, 0.5 = 2-frame history
where t = rejection_score
```

Rejection score ∈ [0,1] comes from the neighbourhood clip in (3); a
score of 0 means history is trusted (low alpha → slow blend → cleaner
image), 1 means history is rejected (high alpha → fast adoption →
respond to disocclusions).

### 3. Neighbourhood-clip rejection (YCgCo space)

For each output pixel, the 3×3 neighbourhood of the *current* frame
in YCgCo color space defines an AABB; the reprojected history sample
is clipped (not clamped) to the AABB. The fraction by which the
sample had to move to enter the AABB drives the rejection score in
(2).

YCgCo over RGB because luminance-chroma separation handles colour-
shifts (e.g. glints) more gracefully — *Karis 2014* documented and
RTR4 endorsed.

### 4. Motion-vector reprojection

`draw.opaque` writes per-pixel motion vectors (`motion+depth` MRT,
spec §IV.4.A line 379). TAA reads the motion vector, samples the
*previous* frame's HDR target at `current_pos - motion`, and blends
it forward. The motion vectors include camera + object motion (the
geometry pass emits both, computed from per-instance
`previous_world_from_object`).

### 5. Ghosting countermeasures

- **Disocclusion mask:** depth ratio between current and reprojected
  depth > 1.1× → reject history fully (alpha → 1.0).
- **Variance-based rejection:** if the 3×3 neighbourhood variance
  collapsed dramatically frame-over-frame (smooth → noisy
  transition), reject. This catches alpha-test foliage popping.
- **Velocity-aware sharpening:** the standard TAA-induced blur is
  countered by a 5-tap unsharp filter applied at the *end* of the
  TAA pass, with strength proportional to `1 − rejection_score`
  (don't sharpen rejected pixels).

### 6. History storage

The history buffer is a single HDR render target (RGBA16F, native
render resolution). Stored across frames by ping-ponging two
allocations in the render-graph (the graph treats them as one
logical `Resource<TaaHistory>` with double-buffering handled by the
resource pool).

## Consequences

- One `Pass` (`TaaPass`) registers as Track::A with
  `reads = [HdrColorTarget(current), TaaHistory(previous), MotionDepth]`,
  `writes = [HdrColorTarget(taa_output), TaaHistory(current)]`.
  Telemetry: `SPAN("post.fx.taa", Subsystem::Render)`.
- The TAA-output target is the canonical input to the
  UpscalerProvider trait (ADR-005). Upscalers always consume TAA-
  resolved frames — DLSS, FSR, XeSS all expect this.
- ADR-040's shadow pass queries the jitter; the contract is "shadow
  cascade jitter == TAA jitter for the same frame index." That
  cross-system invariant is documented here and there.
- Static-scene convergence is 8 frames (~133 ms at 60 FPS) to a
  Halton-covered uniform sample. Camera motion resets convergence
  to a few frames depending on motion magnitude.

## Risks and tradeoffs

- **YCgCo color space costs an RGB↔YCgCo conversion per neighbour
  sample** (9 samples × 2 conversions = ~50 mul-adds per pixel).
  Negligible vs. the gather cost; not a bottleneck.
- **Disocclusion mask cutoff (1.1× depth ratio) is heuristic.** Too
  tight → false rejections on slanted surfaces. Too loose → ghosting
  on actual disocclusions. The 1.1× value matches Karis 2014 and
  the RTR4 recommendation; tunable.
- **Foliage / alpha-test will still flicker.** TAA does not solve
  alpha-test; alpha-to-coverage at the geometry stage is the
  complementary technique (out of TAA's scope).
- **Sharpening at the end re-introduces aliasing on fully resolved
  edges.** Mitigated by tying sharpening strength to rejection
  score (don't sharpen rejected pixels); the residual is the
  industry-standard trade-off.
- **History buffer reads-then-writes against itself.** Double-
  buffering via the render graph keeps this hazard-free as long as
  the pass declares the two slots correctly.

## Alternatives considered

- **SMAA T2x.** Excellent quality, but it's not the GPU-friendly
  pipeline RTR4 ch. 5 documents as the modern standard, and
  upscalers expect a TAA-style input. Rejected.
- **DLAA (no TAA, vendor anti-aliasing).** A vendor solution; the
  spec requires an owned baseline. Rejected.
- **No TAA, MSAA only.** MSAA at 4x is bandwidth-prohibitive on the
  RX 580 milestone target. Rejected.
- **Halton period 16 + Gaussian re-weighting.** Smoother static
  image, more ghosting. Phase 6+ candidate for high-quality preset.

## Verification

- Implementation lands with Phase 5 PR 4. Tests:
  - `tests/taa_jitter_determinism.rs`: `jitter_for_frame(0..1000)`
    must be byte-identical on x86-64 and aarch64. Halton sequence
    uses engine-math arithmetic (no libm), so cross-arch parity
    holds by construction.
  - `tests/taa_neighbour_clip.rs`: synthetic YCgCo neighbourhood
    fixtures verify the clip math against hand-computed expected
    values.
  - `tests/taa_pixel_parity.rs`: rasterizer-testbed reference scene
    with a known camera motion path; CPU TAA implementation in
    `engine-raster` vs GPU implementation within ADR-046 threshold
    *after* the 8-frame convergence window.
- Telemetry: per-frame `GAUGE "taa.rejection_score_p50"` and
  `GAUGE "taa.rejection_score_p99"` so the rejection behaviour
  is observable in profiling.
- The cascade-jitter cross-check is a runtime invariant: a
  `debug_assert_eq!` in the shadow pass against the TAA jitter
  query for the same frame.
