# ADR-041 — IBL · L2 SH probe generation and sampling

- Status: Accepted (Phase 5 design contract; implementation lands in
  Phase 5 PR 4)
- Date: 2026-05-24
- Phase: 5 — RENDERING FOUNDATION (Track A)
- Companion: ADR-039 (render graph), ADR-042 (TAA), ADR-046 (oracle
  regression criteria)

## Context

Spec §IV.4.A line 382 declares the IBL pass: "L2 SH probe sampling,
128 probes baseline." Three terms — L2, SH, 128 — fix the encoding
basis and the probe count but leave open:

- How probes are generated (offline bake? runtime capture? mixed?).
- The cube-map capture resolution and filtering pipeline.
- How probes are spatially indexed (uniform grid? sparse? per-volume?).
- How sampling interpolates between probes.
- The split-sum approximation for the specular term (Karis 2013) and
  its BRDF LUT.

The Karis split-sum is the de-facto industry approximation, documented
in *Real-Time Rendering* 4th ch. 10 and *Physically Based Rendering*
4th ch. 8. The choice space narrows quickly.

## Decision

### 1. Probe generation — offline bake into the pak

Probes are baked by a `engine-shader`-style sandboxed subprocess
tool, `engine-ibl-bake` (lands later — Phase 5 PR 4 ships a CPU stub
in `engine-raster` that produces the same SH coefficients, so the
runtime path can be implemented and tested without the bake tool).
The bake tool:

1. Renders a 256×256×6 cube-map per probe location using the
   rasterizer-testbed CPU path (deterministic; ADR-046 oracle).
2. Convolves the cube-map with the L2 SH basis (nine RGB
   coefficients per probe: 1 + 3 + 5 = 9 bands, banded 0/1/2).
3. Pre-filters a mip chain of the cube-map for the specular split-sum
   term (5 mip levels at 256 / 128 / 64 / 32 / 16).
4. Emits a `IblProbeSet` asset (one per scene), content-addressed via
   the ADR-008 pak pipeline.

The L2 SH basis encodes irradiance to within ~3% of ground truth for
diffuse-only lighting (the standard result from Ramamoorthi-Hanrahan
2001). That's enough for Phase 5; higher orders are a Phase 6
research candidate.

### 2. Probe count and spatial index

128 probes is the baseline (per spec). Probes live on a sparse 3D
grid, keyed by world-space cell `(cx, cy, cz)` with a default cell
size of 4 meters (configurable per scene). The grid is stored as
`engine_core::collections::HashMap<(i16,i16,i16), ProbeId>`
(DeterministicHasher — same cross-arch insurance as the ECS resource
map).

Cells without probes fall back to the global fallback probe (an
all-black L2 + sky color, painted in editor — Phase 10 work). For
Phase 5 a single fallback probe ships hard-coded with a neutral
ambient.

### 3. Probe sampling — trilinear over 8 grid neighbours

For each surface fragment, the lighting accumulation pass
(`draw.opaque.2`, spec §IV.4.A line 383) computes the 8 grid
neighbours of the fragment's world position and trilinearly
interpolates the 9 SH coefficients. Missing neighbours contribute
the fallback probe.

```glsl
// Pseudocode in Slang
let cell = floor(world_pos / probe_cell_size);
let local = fract(world_pos / probe_cell_size);
let weights = trilinear_weights(local);
var sh: ShL2;
for n in 0..8 {
    let probe = probe_at(cell + offset(n));
    sh = sh + weights[n] * probe.sh_coeffs;
}
let diffuse_irradiance = sh_l2_evaluate(sh, surface_normal);
```

Trilinear interpolation is cheap (8 lookups per fragment) and is the
RTR4-documented baseline. More sophisticated probe interpolation
(tetrahedral, RBF) is a Phase 6+ research candidate.

### 4. Specular IBL — Karis split-sum

For the specular term the pass uses the standard split-sum
approximation: a per-probe pre-filtered cube-map (5 mip levels indexed
by roughness²) plus a 2D BRDF LUT (512×512, baked once, ships in the
pak). The BRDF LUT is the same for every scene; the cube-map varies
per probe.

`L_specular(view, normal, roughness, F0)
   = prefiltered_cube(reflect(view, normal), roughness)
   · brdf_lut(N·V, roughness) · F0
   + brdf_lut.g`

### 5. Asset layout

```
IblProbeSet := { probes: [Probe; N <= 128] }
Probe       := {
    position:  Vec3,
    sh_coeffs: [Vec3; 9],         // 9 RGB SH coefficients, L2 basis
    specular:  Handle<CubeMip>,   // 256² + 4 mip levels, BC6H_UFLOAT
}
brdf_lut:    Handle<Texture2D>    // 512×512, RG16F, single asset per engine version
```

Per ADR-045 (texture compression fallback): the per-probe cube mip
chain ships as BC6H by default; uncompressed RG16F is the fallback on
GPUs without BC6H.

## Consequences

- One render-graph pass (`IblPass`) registers as Track::A with
  `reads = [IblProbeSet, GBuffer{Albedo,Normal,RoughMetal}]`,
  `writes = [HdrColorTarget]` (it accumulates into the lighting
  buffer alongside CSM and cluster lights — `draw.opaque.2`).
- Memory: 128 probes × (9 × 12 B + cube mip chain (5 levels of
  BC6H 256² + 128² + 64² + 32² + 16² ≈ 175 KiB)) ≈ 22.5 MiB per
  scene. Modest.
- The bake tool's CPU SH convolution matches the spec's R-02
  oracle pattern: software path is the oracle for the (eventual) GPU
  bake-tool variant.
- TAA jitter (ADR-042) interacts: trilinear probe sampling is stable
  across jitter offsets because the world-space position is sub-
  pixel-stable to the jittered camera projection. No special TAA
  rejection logic needed for IBL.

## Risks and tradeoffs

- **L2 SH bands the diffuse term to ~3% error.** Edge cases with
  strong directional lighting (a setting sun) lose energy. The
  Phase-6 candidate upgrade is L3 (16 coefficients) or replacing SH
  entirely with spherical Gaussians.
- **128 probes is a small scene budget.** A 30 m × 30 m × 10 m
  building with 4 m cells holds ~70 probes — fits. A 200 m × 200 m
  outdoor area at 4 m cells overflows; cells default to fallback
  there. The cell size is per-scene tunable but globally fixed —
  the Phase-6 candidate is per-volume cell density.
- **Bake-tool dependency.** Phase 5 PR 4 needs a CPU bake stub so
  the runtime can be tested. The actual editor-driven bake UI is
  Phase 10. Until then, scenes are baked via a CLI invocation that
  ships in PR 4.
- **Probe lookup is 8 hashmap probes per fragment.** A flat array
  indexed by linear cell coordinate is faster; rejected because
  sparse scenes (most of them) waste memory in a dense array. The
  hashmap is the runtime data structure; the bake tool may emit a
  sorted-index variant in a later iteration.

## Alternatives considered

- **DDGI (Dynamic Diffuse Global Illumination, Majercik 2019).** Real-
  time probe updates via screen-space ray tracing. Excellent
  results; requires ray tracing hardware. Phase 6+ (when 3DGS
  research lands), not Phase 5.
- **Lightmap UV unwrapping with baked GI.** The classical solution.
  Reject: requires per-asset UV unwrapping pipeline (Phase 10
  editor work).
- **Voxel cone tracing.** Phase 6+ research candidate; out of Phase
  5 scope.
- **Pure screen-space GI (SSGI).** Limited to visible surfaces;
  loses occluded light. Used as a *complement* to probe IBL in
  Phase 6+, not a replacement.

## Verification

- Implementation lands with Phase 5 PR 4. Tests:
  - `tests/ibl_sh_convolution.rs` (lands with the CPU bake stub):
    asserts the L2 SH coefficients derived from a known analytic
    light field match the Ramamoorthi-Hanrahan closed-form solution
    to within 1e-4. Cross-arch deterministic (engine-math
    transcendentals).
  - `tests/ibl_trilinear_probe_eval.rs`: deterministic interpolation
    fixture; same input → same output.
  - `tests/ibl_pixel_parity.rs`: rasterizer-testbed reference scene
    with a known IBL probe set; CPU path vs GPU path within ADR-046
    threshold.
- Telemetry: `SPAN("ibl.lookup", Subsystem::Render)` per fragment
  group; `GAUGE "ibl.probe_count"` per frame.
- Determinism: the bake tool's SH convolution uses engine-math
  transcendentals (no libm), so cross-arch byte-equality is preserved.
  Golden file: `crates/engine-render/tests/goldens/ibl_l2_basis.golden`.
