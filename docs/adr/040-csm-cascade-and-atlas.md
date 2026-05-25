# ADR-040 — CSM cascade selection and atlas layout

- Status: Accepted (Phase 5 design contract; implementation lands in
  Phase 5 PR 3)
- Date: 2026-05-24
- Phase: 5 — RENDERING FOUNDATION (Track A)
- Companion: ADR-039 (render graph), ADR-043 (cluster lights),
  ADR-046 (oracle regression criteria)

## Context

Spec §IV.4.A line 380 declares the shadow pass: "CSM, 4 cascades,
4096² D32F atlas." That fixes the cascade count, the atlas extent,
and the depth format. It does *not* fix:

- How the four cascades are partitioned (uniform? logarithmic?
  practical-split blend?).
- How the 4096² atlas is sub-divided across the cascades (one
  2048×2048 quadrant per cascade? variable layout?).
- The shadow-map sampling filter (hardware PCF? PCSS? something
  cheaper?).
- The light-space jitter scheme that interacts with the TAA jitter
  from ADR-042.

Phase 5 PR 3 needs a concrete answer. Industry standard since
~2010 is the **practical-split** scheme (Lloyd-Lefohn-Pellacini), a
logarithmic/uniform blend, and that's what *Real-Time Rendering* 4th
ch. 7 documents as the modern baseline. The audit traced no prior
reasoning in the repo, so this ADR records the choice.

## Decision

### 1. Practical-split cascade selection (λ-blended)

```
split_i = λ · z_near · (z_far / z_near)^(i / N)
       + (1 − λ) · (z_near + (z_far − z_near) · (i / N))
```

with `N = 4`, `λ = 0.6` (default), exposed in the renderer config so
art-direction can tune. The blend matches the spec's stated cascade
count of 4 and gives a near-cascade resolution density appropriate to
character-scale geometry without over-stretching the far cascade.

### 2. Atlas layout — four equal 2048×2048 quadrants

```
+--------+--------+
| C0 NW  | C1 NE  |     C0 covers split[0..1]  (near)
+--------+--------+     C1 covers split[1..2]
| C2 SW  | C3 SE  |     C2 covers split[2..3]
+--------+--------+     C3 covers split[3..4]  (far)
4096 × 4096 D32F        Pixel pitch:
                          C0 ≈ 5 mm    @ split[0]=0.1 m, split[1]=5 m, fov=60°
                          C3 ≈ 30 cm   @ split[3]=200 m, split[4]=1000 m
```

Fixed quadrant layout (not virtual / variable) for simplicity and
because the cascade count is fixed. The light-space view-projection
matrices are recomputed per frame; the atlas allocation itself is
static at startup.

### 3. Depth format: D32F (per spec)

D32F is reverse-Z friendly (the engine uses reverse-Z: 1.0 at near,
0.0 at far) and gives uniform precision across the 0..1 range, which
matters for the far cascade more than for the near. D24X8 was
considered and rejected: the precision win at near range (where
shadows are crisp anyway) does not offset the precision loss at far
range (where shadows are most prone to peter-panning).

### 4. PCF + cascade-aware jitter

Sampling: 5×5 PCF with a Vogel-disk kernel (16 taps), per-pixel
rotated by a screen-space hash. Cheaper than PCSS, leagues better than
2×2 hardware PCF for character shadows.

Light-space jitter: the cascade view-projection includes a sub-texel
jitter that *matches the TAA jitter* (ADR-042). Without this, TAA
re-projects across cascade splits and produces flicker on shadow
edges; with it, TAA history aligns and converges to a stable image.
This is the cascade ↔ TAA contract: ADR-042's jitter pattern must be
queryable per frame, and the shadow pass consumes it.

### 5. Snap-to-texel and tight bounding

Each cascade's frustum slice projects to a tight orthographic AABB in
light space. The AABB is snapped to texel grid (per-pixel pitch) to
prevent shimmering when the camera translates. Standard idTech /
Frostbite practice; *RTR4* ch. 7 documents the algorithm.

## Consequences

- Single `Pass` struct (`CsmShadowPass`) implements `render_graph::Pass`
  (ADR-039) with `writes = [ShadowAtlas]`, `reads = [RenderQueue,
  ShadowCasters]`. Telemetry: one `SPAN("shadow", Subsystem::Render)`
  per cascade.
- Memory: one persistent `ShadowAtlas` resource (64 MiB, 4096×4096×4B
  = 64 MiB). Not free, but the bindless heap (ADR-044) is the larger
  cost.
- The 5-mm-near to 30-cm-far pixel-pitch envelope above is the design
  envelope. Anything finer needs more cascades (out of spec) or PCSS
  (deferred to Phase 6+).
- TAA must publish a per-frame jitter offset query API (`taa_jitter
  (frame_idx) -> Vec2`); the shadow pass reads it. ADR-042 records
  the producer side.

## Risks and tradeoffs

- **Fixed quadrant layout means cascades cannot trade resolution.**
  If the far cascade is wasted (camera always looking down a
  corridor) the resolution is locked. A virtual texture atlas could
  redistribute, but that's Phase 6+ work. Acceptable trade for Phase
  5.
- **Vogel-disk PCF is 16 taps per shadow sample.** On the RX 580
  milestone (60 FPS @ 1440p) this lands inside budget per spec's
  ch. 7 cost model (~0.4 ms for character shadows at 1440p), but it
  is the largest single shadow cost. The PCF tap count is configurable
  (low / med / high quality presets) for downstream tuning.
- **Reverse-Z dependency.** If a future Phase chooses to drop reverse-Z
  (unlikely), the cascade math has to flip. Documented here as a
  cross-system invariant.
- **λ = 0.6 is a defaulting choice, not a proof.** Empirically
  reasonable across the RTR4 / Frostbite literature; tunable. The
  rasterizer-testbed reference scenes (ADR-046) include a shadow-
  heavy fixture that pins regressions if λ drifts later.

## Alternatives considered

- **Variance Shadow Maps (VSM) / Exponential Shadow Maps (ESM).**
  Better filtering, but light-bleed (VSM) and ringing (ESM) require
  game-specific tuning. Rejected: CSM + PCF is the predictable
  baseline.
- **Cascaded Shadow Maps Plus (CSM+).** A modern variant that
  computes cascade splits from the depth buffer per-frame.
  Performance gain is real; complexity gain is also real. Phase 6+
  candidate.
- **Per-light shadow maps without cascades** (one map per light).
  Rejected for sun/main directional; appropriate for spot/point
  lights, which use a separate 1024×1024 shadow atlas (out of this
  ADR's scope — Phase 5 PR 3 adds point/spot shadows as a follow-up
  if budget permits, else Phase 6).

## Verification

- Implementation lands with Phase 5 PR 3. Test harness:
  `tests/csm_atlas_layout.rs` asserts the four cascade
  view-projections are computed deterministically (same seed →
  same matrices, byte-equal); `tests/csm_atlas_pixel_parity.rs`
  renders the rasterizer-testbed reference shadow scene against the
  software path and asserts within-ADR-046 threshold.
- Telemetry: every cascade's pass time is a `SPAN`. A `GAUGE
  "shadow.cascade_pixel_pitch_m"` is emitted per frame per cascade
  so the design envelope is visible in profiling.
- No CI guard specific to shadows — the wider `wgpu::` and
  determinism guards already cover the layer.
