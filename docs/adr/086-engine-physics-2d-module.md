# ADR-086 — engine-physics 2D module

- Status: Accepted (planning record; implementation lands in Phase 7 PR 3)
- Date: 2026-05-29
- Phase: 7 — PHYSICS + 2D (Engine Core v0.5)
- Companion: ADR-085 (3D solver this module shares), ADR-089 (tilemap
  collision-rect source), ADR-013 (determinism), ADR-033 (replay
  oracle), spec §IV.451 / §IV.7, milestone spec line 1642

## Context

The Phase-7 milestone (spec line 1642) is a **2D platformer running
with correct physics and 2D lighting**. "Correct physics" is the
load-bearing clause: the character controller must resolve against
tilemap collision rects, stack/push dynamic bodies, and run a rope.
Under the Hybrid fidelity target the 2D path is **spec-faithful** —
this is the milestone, not portfolio breadth — so the 2D solver,
tick, and determinism match the spec exactly.

The 2D module lives on the same crate as the 3D solver (ADR-085) and
reuses its constraint solver and persistent-manifold representation,
specialised to two dimensions. The plan's directive: "reuses the 3D
contact manifold shape."

## Decision

### 1. `r2d.rs` — the 2D world

A `World2d` holding 2D `RigidBody2d` SoA (`pos: Vec2`, `rot: f32`,
`linvel: Vec2`, `angvel: f32`, `inv_mass`, `inv_inertia: f32`). Shares
`tick.rs` (120 Hz fixed, ≤3 substeps) and
`solver/sequential_impulses.rs` (8-iter PGS) with the 3D path — the
solver is generic over a constraint-row trait, and 2D contributes 2D
rows (1 normal + 1 tangent per contact point).

### 2. Colliders + narrow-phase — SAT + clipping

2D colliders: `Circle | Obb | Capsule2d | ConvexPoly2d`. Narrow-phase
is the **separating-axis test** with Sutherland–Hodgman reference-face
clipping (Ericson ch. 5; Box2D-Lite manifold generation), producing a
≤2-point manifold in the shared `manifold.rs` representation. SAT is
chosen over GJK/EPA for 2D because it directly yields the clipped
2-point manifold the solver needs; GJK/EPA stays the 3D path. Broad
phase reuses `broadphase/sap.rs` on 2D AABBs.

### 3. Verlet rope (XPBD-rope analog)

`Rope2d` is a chain of point masses with distance constraints solved
by **Verlet integration + Gauss-Seidel constraint relaxation** (the
deterministic position-based analog of the spec's XPBD rope). This is
the one soft-body primitive v0.5 ships (per the ADR-085 deviation note
deferring full XPBD cloth/soft-body). Rope endpoints can pin to a
dynamic body or a world anchor.

### 4. Tilemap collision surface

`engine-tilemap` (ADR-089) emits per-chunk **collision rectangles**;
`World2d::add_static_tilemap()` ingests them as immovable `Obb`
colliders keyed by chunk so streaming (chunk load/unload) adds/removes
static colliders without re-solving the whole world. This is the
character-vs-level path the platformer exercises.

### 5. Determinism

Identical contract to ADR-085: stable `BodyId` iteration, FMA-off sim
profile, RNG only for reproducible tie-breaking (shares the
`physics.*` channels). The `tilemap_collide_2d` replay fixture pins a
character falling onto and walking across a tilemap.

## Rationale

- **Share the solver, specialise the geometry.** The solver is the
  spec's named algorithm and the hard determinism path; writing it
  once for both dimensions avoids two divergent impulse loops.
- **SAT for 2D** is the standard manifold generator and yields exactly
  the 2-point clipped manifold the PGS rows consume — simpler and more
  robust in 2D than running GJK/EPA then re-deriving contact points.
- **Verlet rope** gives the platformer a rope/chain (swing, bridge)
  deterministically without the XPBD compliance machinery.

## Consequences

- No new crate — extends `engine-physics`.
- `engine-render-2d` (ADR-088) consumes the debug-draw buffer; the
  platformer starter-kit (ADR-100) wires character control on top.
- The 2D static-collider ingest is the integration seam tested by
  `tilemap_collide_2d` and the platformer golden (ADR-100).

## Risks and tradeoffs

- **SAT axis enumeration is O(edges²) for two convex polys.** Bounded
  by the small vertex counts (boxes, ≤8-gons) v0.5 uses; circles and
  capsules use analytic axes. Accepted.
- **Verlet rope is not momentum-exact.** Accepted — it is a gameplay
  rope, not a simulation-grade cable; constraint-relaxation iteration
  count is tuned for visual stability and pinned by replay parity.

## Alternatives considered

- **A separate 2D solver loop.** Rejected — duplicates the spec's PGS
  and doubles the determinism surface.
- **GJK/EPA for 2D contacts.** Rejected — extra step to recover the
  contact manifold SAT gives directly.
- **Full XPBD rope.** Rejected for v0.5 — Verlet+relaxation is the
  deterministic, simpler primitive that meets the milestone.

## Verification

- `crates/engine-physics/tests/replay_parity.rs` — `chain_settle`
  (rope) + `tilemap_collide_2d` (character vs tilemap) BLAKE3 digests
  match across two runs.
- Unit tests: SAT manifold against analytic box-box overlap; circle-vs-
  box contact point/normal; rope rest-length convergence.
- The platformer golden (ADR-100) is the end-to-end milestone gate.
- `just ci` green at the PR-3 commit.
