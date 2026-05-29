# ADR-088 — engine-render-2d Level-2 crate

- Status: Accepted (planning record; implementation lands in Phase 7 PR 5)
- Date: 2026-05-29
- Phase: 7 — PHYSICS + 2D (Engine Core v0.5)
- Companion: ADR-040 (deferred renderer — reverse-Z convention this
  matches), ADR-046 (pixel-parity oracle + §6b fixture), ADR-075
  (pass `record()` discipline), ADR-089 (tilemap chunk source),
  ADR-051 (three deviations), spec §IV.431, milestone spec line 1642

## Context

The milestone (spec line 1642) requires **correct 2D lighting**. The
spec's 2D render graph (§IV.431) is:

> `sprite.batch` (GPU-instanced, one draw call per atlas per layer) →
> `tilemap.chunk` (chunk-based, GPU index-texture lookup) → `lighting2d`
> (deferred 2D, normal-map diffuse + stencil shadow volumes) →
> `shape.batch` → `sdf.text` → `post.fx.2d`.

Under the Hybrid fidelity target, the **milestone-load-bearing stages
ship spec-faithfully**: `sprite.batch`, `tilemap.chunk` (rendered from
`engine-tilemap`, ADR-089), `lighting2d` (deferred-2D with normal-map
diffuse **and stencil shadow volumes**), and the composite edge into
the 3D scene color. The remaining stages (`shape.batch`, `sdf.text`,
`post.fx.2d`) are 2D-renderer breadth the platformer does not need and
are recorded as ADR-051 deviations rather than shipped in v0.5.

## Decision

### 1. Crate layout (Level 2)

```
crates/engine-render-2d/src/
  lib.rs            — Renderer2d, public surface
  sprite_batcher.rs — per-atlas batch builder, deterministic submit
  camera_2d.rs      — orthographic + reverse-Z (matches engine-render)
  lighting_2d.rs    — deferred-2D G-buffer + per-light + stencil shadows
  composite.rs      — render-graph edge: 2D over/under deferred-3D
```

Deps: `engine-math`, `engine-platform`, `engine-core`, `engine-asset`
(atlas textures), `engine-render` (graph + `PassContext` + reverse-Z
constants + G-buffer formats).

### 2. Sprite batching — deterministic submit order

`sprite_batcher.rs` bins sprites by texture-atlas page and issues one
GPU-instanced draw per `(layer, atlas)`. Submit order is sorted by
`(layer, atlas_id, sort_key, EntityId)` — fully deterministic, so the
oracle (below) is reproducible. Instance data is SoA (transform, UV
rect, color, normal-map slice).

### 3. Camera — orthographic reverse-Z

`camera_2d.rs` builds an orthographic projection using the **same
reverse-Z depth convention** as `engine-render` (ADR-040), so 2D depth
(layer) composites correctly against the 3D depth buffer at the
composite edge. Layer → depth is an explicit, stable mapping.

### 4. Deferred-2D lighting (spec-faithful, milestone gate)

`lighting_2d.rs` runs the spec's `lighting2d` design:
1. **2D G-buffer**: sprite/tilemap pass writes albedo + a 2D normal
   (from the optional normal-map slice; flat `+Z` when absent) +
   depth/layer.
2. **Per-light accumulation**: each 2D light (point/spot/directional)
   adds `albedo · N·L · attenuation` — normal-map diffuse, additively
   blended.
3. **Stencil shadow volumes**: occluder silhouette edges are extruded
   away from each light into a stencil mask; lit accumulation is
   gated by the stencil so occluders cast hard 2D shadows.

Shaders: `sprite.wgsl` (G-buffer write), `light_2d_deferred.wgsl`
(per-light accumulate), `shadow_volume_2d.wgsl` (silhouette extrude +
stencil). Each follows the ADR-075 6-step `record()` shape.

### 5. Composite

`composite.rs` adds a render-graph edge compositing the 2D lit target
over (HUD/foreground) or under (parallax background) the deferred-3D
scene color, selected per-layer, before TAA reads scene color.

## Rationale

- **Spec-faithful 2D lighting** is the literal milestone clause; the
  deferred-2D + normal-diffuse + stencil-shadow path is what "correct
  2D lighting" means in §IV.431, so it ships as written.
- **Deterministic submit order** makes the `sprite_batch` pixel-parity
  oracle (ADR-046 §6b) reproducible — the same discipline the 3D
  fixtures use.
- **Reverse-Z reuse** is non-negotiable for the composite to depth-test
  correctly against the 3D path.

## Consequences

- New Level-2 crate; `Cargo.toml` gains `engine-render-2d`.
- `engine-render` registers three new WGSL shaders + smoke tests and
  grows a graph edge for the 2D composite.
- ADR-046 gains a §6b amendment defining the `sprite_batch` fixture.
- ADR-051 gains three deviation entries: `shape.batch` (debug
  primitives), `sdf.text` (MSDF font rendering), `post.fx.2d` (CA /
  vignette / palette-swap) not implemented in v0.5. No future-phase
  tag — recorded deviations, not silent drops.

## Risks and tradeoffs

- **Stencil shadow volumes** add a stencil pass and silhouette
  extraction per light. Accepted — it is the spec's named technique
  and the milestone gate; light count in the platformer is small.
- **2D/3D composite depth interplay** is subtle (reverse-Z, layer→depth
  mapping). Mitigated by reusing engine-render's exact constants and a
  composite ordering test.

## Alternatives considered

- **Forward additive 2D lighting (no G-buffer, no shadows).** Rejected
  — it is not the spec's `lighting2d`; it would not satisfy "correct
  2D lighting" (no normal-map diffuse interplay, no shadows).
- **Folding 2D into engine-render.** Rejected — 2D is a distinct graph;
  a sibling Level-2 crate (like engine-splatting) keeps the boundary
  clean and the wasm32 target (ADR-092) able to select it independently.

## Verification

- `crates/engine-render-2d/tests/sprite_batch_parity.rs` — ADR-046 §6b
  `sprite_batch` fixture: GPU output vs CPU sprite oracle within the
  oracle threshold.
- `engine-render` shader smoke tests for the three WGSL shaders.
- The platformer golden (ADR-100) exercises lighting2d + stencil
  shadows end-to-end — the milestone gate.
- `just ci` green at the PR-5 commit.
