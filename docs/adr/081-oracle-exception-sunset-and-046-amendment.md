# ADR-081 — Oracle exception sunset plan + ADR-046 amendment

- Status: Accepted (Phase 6 design contract; implementation lands in
  Phase 6 PR 1a)
- Date: 2026-05-28
- Phase: 6 — NEURAL RENDERING & GAUSSIAN SPLATTING
- Companion: ADR-046 (rasterizer oracle regression — amended by
  this ADR), ADR-077 (3DGS architecture — uses the new
  architectural-divergence category), ADR-084 (Phase 6 PR slicing)

## Context

Phase 5.5 closed (2026-05-28) with six active entries in
`docs/audit/oracle-exceptions.md`:

| Fixture | Status before Phase 6 |
|---|---|
| `cube` | driver-level f32 drift on Mesa RADV (vendor category) |
| `csm_4_cascade` | engine-fix: WGSL `_shadow` hook is unused; cascade projection + atlas sample missing in shader |
| `cluster_64_lights` | engine-fix: point-light attenuation kernel mismatch (pure `1/dist²` vs ADR-043 windowed) |
| `ibl_probe` | engine-fix: BRDF LUT placeholder; helper `bake_brdf_lut` exists but harness does not call it |
| `taa_motion` | engine-fix: inherits cube floor; "structural" cleanup is to align with cube's exception bound |
| `post_fx_chain` | structural: CPU oracle is single-pass full-res; GPU is multi-pass half-res SSAO + 5-mip bloom — per-pixel parity is design-inappropriate |

Per ADR-046 §3 the register exists to formalise *driver / vendor*
divergences, not to absorb engineering debt. Four of the six
entries are engine-fixes (the WGSL shaders + the BRDF LUT bind are
*incomplete*, not divergent). One (`cube`) is a real vendor-driver
exception. One (`post_fx_chain`) is an *architectural* divergence:
the CPU oracle and the GPU path do *fundamentally different work*,
not because of vendor drift, but because the two design intents
differ (single-pass reference vs. multi-pass production).

ADR-046's three categories — *engine bug*, *CPU oracle out-of-date*,
*vendor-specific* — do not name this fourth category. Either
`post_fx_chain` is mis-categorised as "vendor-specific" (it isn't —
the divergence persists on every driver because the algorithms
differ), or ADR-046 needs a new category.

This ADR formalises both:

1. **Sunset 4 entries** by fixing the engine in PR 1a.
2. **Amend ADR-046** to add the *architectural divergence* category.
3. **Convert `post_fx_chain`** from a follow-up to a permanent
   architectural-divergence exception under the new category.

## Decision

### 1. Four sunset entries (engine-fixes land in PR 1a)

| Fixture | Engine fix |
|---|---|
| `csm_4_cascade` | `crates/engine-render/shaders/lighting.wgsl` line 141 — the `_shadow` hook becomes a real CSM cascade projection + atlas sample. Math ported from `testbed/engine-raster/src/shadow.rs::sample_shadow_pcf`. |
| `cluster_64_lights` | `crates/engine-render/shaders/lighting.wgsl` point-light attenuation kernel becomes the ADR-043 windowed inverse-square `(1 - clamp(d/range, 0, 1))² / max(d², 1)`. |
| `ibl_probe` | `crates/engine-render/tests/pixel_parity/ibl_probe.rs` harness calls `engine_render::init::bake_brdf_lut(device, queue)` and binds the real LUT instead of the placeholder. |
| `taa_motion` | No engine change. The exception's "structural cleanup" body is realised by the cube fixture's exception band staying in place — `taa_motion` inherits via the documented cube-floor inheritance. Register row moves to *Sunset* with the rationale "inherits cube floor; closure is documentation, not code". |

After PR 1a, the *Active* table contains 2 entries (`cube`,
`post_fx_chain`) and the *Sunset* table contains 4 entries (the
above).

### 2. ADR-046 amendment — *architectural divergence* category

ADR-046 §3 is amended (the amendment lands in PR 1a as a section
edit to `docs/adr/046-rasterizer-oracle-regression.md`) to add a
fourth category to the existing three:

> 4. **Architectural divergence.** The CPU oracle and the GPU path
>    do *fundamentally different work* — different number of passes,
>    different intermediate resolutions, different kernel shapes —
>    because the two design intents differ. The CPU oracle is the
>    *reference* implementation (single-pass, full-resolution,
>    scalar f32) targeted at numerical clarity for tests and
>    debugging; the GPU path is the *production* implementation
>    (multi-pass, mip-pyramid, hardware-accelerated) targeted at
>    real-time performance. The fixture verifies the *wiring*
>    (every pass executes; the chain produces a non-trivial output)
>    rather than per-pixel parity. Bound: SSIM ≥ 0.85 minimum;
>    fixture-specific lower bound documented per-fixture.

The category exists for *intentional* divergences. ADR-077's 3DGS
fixtures (`splat_garden_1m`, `splat_view_dependent` composite) join
this category — they verify the sort + composite wiring at SSIM
≥ 0.95, with the inherent blend-order f32 precision drift cited
as the architectural reason the per-pixel bound is loosened.

### 3. Convert `post_fx_chain` to permanent architectural exception

Register row's rationale shifts from "post-v0.3 follow-up" to:

> Architectural divergence (ADR-046 category 4 per ADR-081's
> amendment). The CPU oracle composes SSAO darkening + bloom-extract
> + tonemap inline at full resolution; the GPU runs SSAO as a
> separate compute pass writing a half-res target, then a 5-mip
> Gaussian-kernel bloom pyramid. The two paths are *not the same
> algorithm* — they are different design points trading off
> reference-clarity vs. production-perf. The fixture's structural
> assertions verify the chain executes end-to-end (every pass runs;
> output is non-zero; histogram is in the expected range). Per-pixel
> parity at 1/255 is structurally outside the oracle's design
> intent. Permanent exception; ADR/PR column points at ADR-081.

The `cube` row stays unchanged (vendor-driver category).

### 4. Register schema change

`docs/audit/oracle-exceptions.md` grows a *Category* column:

| Fixture | Category | Violation | ... |

with categories `engine-fix`, `vendor-driver`, `architectural`,
`cpu-oracle-stale`. Sunset entries name the sunset PR / date in
their *Sunset* row.

### 5. ADR-077 3DGS fixture inheritance

`splat_garden_1m` and `splat_view_dependent` (composite path)
register entries name *architectural* as their category from
the moment they land. The fixtures' assertion is SSIM ≥ 0.95 on
the composite; strict 1/255 on the per-fragment math
(`splat_view_dependent` SH evaluation alone hits strict).

### 6. Workflow update

ADR-046's *Workflow* section (the four-step Detection /
Investigation / Decision / Sunset flow) grows the
*Architectural* branch under Decision:

> - **Architectural divergence** → add an entry here under category
>   *architectural*; document the design intent split (which
>   path is reference, which is production); the entry is
>   *permanent* and never sunsets unless one of the two paths is
>   redesigned to match the other.

This makes the new category visible in the register-maintenance
ritual.

## Consequences

### Positive

- The exception register stops conflating engineering debt with
  legitimate divergence categories.
- Four oracle exceptions sunset to strict 1/255 in PR 1a; the
  fixtures' rigor strengthens (regressions are now visible at the
  default threshold).
- ADR-046's category set is honest about the reference / production
  split inherent in any GPU engine.
- ADR-077's SSIM-band fixtures inherit a documented category, not
  a special-cased fixture-specific carve-out.

### Negative

- ADR-046 is amended (not superseded). The ADR set has multiple
  ADRs with amendments (ADR-051 living register, ADR-067 third
  amendment); the amendment pattern is established precedent.
- The *architectural* category could be abused to soft-pedal real
  engine bugs. Mitigated by ADR-046 §3's existing review rubric:
  each entry must name the *design intent split* and cite the
  algorithmic difference. A shallow PR ("ADR-046 category 4
  divergence: see fixture") is visibly shallow.

### Neutral

- The category column in the register table is forward-compatible
  (older parsers ignore it; current readers gain category info).

## Implementation

PR 1a of Phase 6 (per ADR-084):

1. `crates/engine-render/shaders/lighting.wgsl` — CSM cascade
   projection + windowed point-light attenuation.
2. `crates/engine-render/tests/pixel_parity/ibl_probe.rs` —
   real BRDF LUT bind.
3. `docs/adr/046-rasterizer-oracle-regression.md` — amend §3 with
   the architectural category.
4. `docs/audit/oracle-exceptions.md` — split active/sunset tables;
   add category column; populate the rationale changes.

## References

### Prior engine ADRs

- [ADR-043](043-cluster-lights-binning.md) — the windowed
  inverse-square attenuation the lighting WGSL must adopt.
- [ADR-046](046-rasterizer-oracle-regression.md) — amended by
  this ADR.
- [ADR-077](077-3dgs-architecture.md) — 3DGS fixtures inherit
  the architectural category this ADR creates.
- [ADR-084](084-phase-6-pr-slicing.md) — Phase 6 PR slicing.
