# ADR-087 — engine-particles Level-2 crate

- Status: Accepted (planning record; implementation lands in Phase 7 PR 4)
- Date: 2026-05-29
- Phase: 7 — PHYSICS + 2D (Engine Core v0.5)
- Companion: ADR-077 (engine-splatting — the sibling SoA+sort+composite
  pattern this mirrors), ADR-046 (pixel-parity oracle), ADR-033
  (replay-parity oracle), ADR-057 (RNG channel `particles`), ADR-075
  (pass `record()` discipline), ADR-051 (one deviation), spec §IV.451

## Context

Phase 7's portfolio (spec line 1640) includes **GPU particles**.
Particles are short-lived alpha-blended sprite quads (sparks, smoke,
debris) — distinct from `engine-splatting`'s 3D Gaussian Splatting
(persistent learned point clouds). The two crates share a *shape*
(SoA storage + depth sort + back-to-front composite) but not a
purpose, so particles get their own Level-2 crate rather than bloating
`engine-splatting`.

The spec (§IV.451) couples particles to physics: "GPU particle physics
via compute shader with BVH read-back." Under the Hybrid target v0.5
ships **standalone compute particles** (forces + lifetime, no
physics-broadphase collision read-back); the coupling is an ADR-051
deviation.

## Decision

### 1. Crate layout (Level 2)

```
crates/engine-particles/src/
  lib.rs         — ParticleSystem, EmitterDesc, public surface
  emitter.rs     — deterministic spawn (rate, lifetime, init distributions)
  simulator.rs   — per-particle force integration (gravity, drag, curl)
  sort.rs        — back-to-front depth sort (mirrors engine-splatting::sort)
  composite.rs   — instanced-billboard render-graph pass contract
```

Deps: `engine-math`, `engine-platform`, `engine-core` (RNG). The GPU
pass binds through `engine-render`'s `PassContext` (the crate ships the
record contract; `engine-render` registers the shaders), mirroring how
`engine-splatting::SplatCompositePass` integrates.

### 2. SoA particle state + emitter

`ParticleState` SoA: `position`, `velocity`, `age`, `lifetime`,
`size`, `color_rgba`, `seed`. The emitter spawns N/step from
`EmitterDesc` (rate, cone/disc/box shape, speed range, lifetime range),
drawing all randomness from RNG channel `particles` keyed by
`(seed, frame, "particles", counter)` so spawns are reproducible.

### 3. Simulator

Per-particle semi-implicit Euler under accumulated forces (constant
gravity, linear drag, optional curl-noise field). Dead particles
(`age ≥ lifetime`) are compacted via a stable partition so live-set
ordering stays deterministic.

### 4. Sort + composite

`sort.rs` reuses the `engine-splatting` radix-sort pattern to order
live particles back-to-front by camera-space depth for correct alpha
blending. `composite.rs` is a `record()`-discipline pass (ADR-075
6-step) issuing one instanced draw of billboard quads. Three WGSL
shaders register in `engine-render`: `particle_simulate.wgsl`
(compute), `particle_sort.wgsl` (compute radix), `particle_composite.wgsl`
(billboard VS/FS).

### 5. Determinism

The CPU simulator path is the determinism reference: replay-parity
(ADR-033) asserts byte-identical SoA state across two runs of a
100 K-particle scene on the same seed. The GPU path is validated by
the ADR-046 oracle pattern (GPU output vs CPU reference within the
oracle threshold), not by bit-exact GPU replay.

## Rationale

- **Separate crate** keeps `engine-splatting`'s 3DGS surface clean and
  lets particles depend on `engine-core` RNG without dragging it into
  the splat path.
- **Mirror the splatting SoA+sort+composite shape** so the sort and
  composite code is a known-good pattern, not a new invention.
- **Standalone forces (no BVH read-back)** meets the visible-particles
  portfolio goal; particle-vs-world collision is breadth the milestone
  does not need.

## Consequences

- New Level-2 crate; `Cargo.toml` `[workspace.dependencies]` gains
  `engine-particles`.
- `engine-render` registers three WGSL shaders + smoke tests.
- ADR-051 gains one deviation entry (no GPU-particle BVH read-back).
- RNG channel `particles` documented in the ADR-057 amendment.

## Risks and tradeoffs

- **GPU/CPU divergence** at the last ULP. Accepted via the ADR-046
  oracle threshold; the determinism guarantee is on the CPU path.
- **Sort cost at 100 K particles.** Radix sort is O(n); the
  `particles_100k` frame-pacing scene pins the budget.

## Alternatives considered

- **Fold particles into engine-splatting.** Rejected — different
  lifetime model, different purpose, would couple 3DGS to RNG.
- **CPU-only particles.** Rejected — the spec says GPU particles; the
  compute path is the deliverable, the CPU path is the oracle.

## Verification

- `crates/engine-particles/tests/replay_parity.rs` — 100 K particles
  bit-exact across two CPU runs.
- `engine-render` shader smoke tests for the three WGSL shaders.
- `crates/engine-particles/tests/pass_record_discipline.rs` (or the
  workspace-level enforcer) confirms the composite pass follows the
  ADR-075 6-step shape.
- `testbed/frame-pacing/scenes/particles_100k.ron` budget row (PR 18).
- `just ci` green at the PR-4 commit.
