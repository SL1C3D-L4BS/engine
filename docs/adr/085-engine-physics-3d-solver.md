# ADR-085 — engine-physics 3D rigid-body solver architecture

- Status: Accepted (planning record; implementation lands in Phase 7 PR 3)
- Date: 2026-05-29
- Phase: 7 — PHYSICS + 2D (Engine Core v0.5)
- Companion: ADR-086 (2D module on this crate), ADR-013 (determinism
  contract — physics is on the per-frame sim budget), ADR-033
  (replay-parity oracle pattern), ADR-057 (BLAKE3 RNG channels),
  ADR-051 (acknowledged deviations — this ADR adds three), ADR-031
  (archetype-SoA ECS the bodies mirror), spec §IV.451 / §IV.7

## Context

Phase 7's portfolio (spec line 1640) opens with **rigid body 3D**.
`crates/engine-physics/` is a 3-line doc-only stub. The spec
(§IV.451) prescribes a specific 3D pipeline:

> AABB BVH (SAH construction, incremental refit) broad-phase; GJK +
> EPA narrow-phase; sequential-impulse PGS solver (8 iterations); XPBD
> for soft bodies/cloth/rope; 120 Hz fixed tick, max 3 substeps per
> render frame; sphere/box/capsule/convex-hull/triangle-mesh/
> heightfield colliders; sensor triggers with enter/exit events; GPU
> particle physics via compute shader with BVH read-back.

The Phase-7 plan locks a **Hybrid (milestone-faithful)** fidelity
target: spec-faithful on the paths the 2D-platformer milestone (spec
line 1642) depends on and on the determinism contract; pragmatic on
3D portfolio breadth, with each divergence recorded as an ADR-051
acknowledged deviation (no future-phase tag). The reference books are
Ericson, *Real-Time Collision Detection* (GJK/EPA, SAP, manifolds)
and Millington, *Game Physics Engine Development* (sequential-impulse
solver, contact resolution).

The hard constraint is ADR-013: the physics step is on the
deterministic sim path. Two runs of the same scene with the same seed
must produce byte-identical state. That rules out any reliance on
unstable sort order, address-dependent iteration, or FMA-contraction
drift (the `sim` profile builds with `-C target-feature=-fma`).

## Decision

### 1. Crate layout (Level 2)

```
crates/engine-physics/src/
  lib.rs              — public surface (PhysicsWorld, Collider, RigidBody)
  world.rs            — RigidBody SoA + island construction + step()
  broadphase/sap.rs   — sweep-and-prune AABB overlap pairs
  narrowphase/gjk.rs  — Gilbert–Johnson–Keerthi boolean + closest point
  narrowphase/epa.rs  — Expanding Polytope Algorithm (depth + normal)
  manifold.rs         — persistent contact manifold (≤4 points, warm-start)
  solver/sequential_impulses.rs — 8-iter PGS, friction + restitution + joints
  ccd.rs              — conservative-advancement continuous collision
  tick.rs             — 120 Hz fixed accumulator, ≤3 substeps/frame
  r2d.rs              — 2D module (see ADR-086)
```

Deps: `engine-math` (vectors/quats/AABB), `engine-core` (RNG +
`TypeStableId` for deterministic ordering), `engine-platform` (fiber
jobs for island-parallel solve). No `engine-render` dependency — the
debug-draw surface is a data buffer the renderer consumes.

### 2. Body storage — SoA, ECS-mirrored

`RigidBody` state is stored SoA (ADR-031 discipline): parallel arrays
for `position`, `orientation` (quat), `linear_velocity`,
`angular_velocity`, `inv_mass`, `inv_inertia_tensor`, `collider_id`,
`flags`. Bodies are addressed by a dense `BodyId(u32)`; the mapping to
ECS `Entity` is a side table so the solver never chases ECS storage.

### 3. Broad-phase — sweep-and-prune (ADR-051 deviation)

Single-axis incremental SAP over body AABBs, axis chosen per-frame by
largest variance. Produces a deterministically-ordered set of
candidate pairs (sorted by `(min(BodyId), max(BodyId))`).
**Deviation from spec §IV.451's AABB-BVH/SAH** — recorded as ADR-051
entry; justified below.

### 4. Narrow-phase — GJK + EPA, convex+primitive colliders

`Collider` = `Sphere | Box | Capsule | ConvexHull`. GJK resolves
boolean overlap and closest-feature; on penetration, EPA recovers
contact normal + depth. **Deviation: triangle-mesh + heightfield
colliders are not shipped in v0.5** — recorded as ADR-051 entry.

### 5. Constraint solver — sequential-impulse PGS, spec-faithful

8 velocity iterations of projected Gauss-Seidel (Catto/Millington),
warm-started from the persistent manifold's accumulated impulses.
Contact constraints (non-penetration + Coulomb friction with a
2-direction tangent basis) and joint constraints (point-to-point,
hinge) share the constraint-row representation. Baumgarte position
bias with a slop; restitution via the relative-normal-velocity
threshold. Iteration count, slop, and bias are spec/Catto-derived
constants in `solver/sequential_impulses.rs`.

### 6. Time stepping — 120 Hz fixed, ≤3 substeps (spec-faithful)

`tick.rs` accumulates render-frame `dt` and runs fixed 1/120 s steps,
capped at 3 substeps/render-frame (spill is dropped to avoid the
spiral-of-death). This is verbatim from spec §IV.451 and is shared by
the 2D module (ADR-086).

### 7. CCD — conservative advancement

Fast bodies (those whose swept AABB this step exceeds their static
AABB by a threshold) get a conservative-advancement TOI pass against
their broad-phase neighbours before the solve. Speculative contacts
feed the manifold so the solver resolves the predicted impact.

### 8. Determinism

No floating RNG in the integrator. RNG (ADR-057) is used only for
**reproducible tie-breaking**: `physics.broadphase` and
`physics.narrowphase` break equal sort keys; `physics.solver.tiebreak`
fixes constraint-row order when two islands hash equal. Island
construction (union-find over contact pairs) iterates bodies in
`BodyId` order. All accumulation is left-fold in `BodyId` order on the
FMA-disabled `sim` profile.

## Rationale

- **SAP over BVH/SAH for v0.5.** The milestone is a 2D platformer; the
  3D path is portfolio breadth proving the solver. SAP is ~150 LOC,
  trivially deterministic (one sorted axis), and O(n) under the
  spatial coherence platformer/demo scenes exhibit. BVH/SAH adds SAH
  partitioning, refit, and tree-rebalance determinism concerns for a
  scene scale v0.5 does not hit. The deviation is recorded, not hidden.
- **8-iter PGS is the spec's solver.** It is the load-bearing
  determinism + correctness path; we implement it verbatim. This is
  also where "correct physics" in the milestone gate is earned.
- **GJK/EPA + convex/primitive colliders** cover every collider the
  platformer + the `rigid_body_3d_100k` bench need. Triangle-mesh /
  heightfield are level-geometry colliders that the 2D milestone does
  not exercise.

## Consequences

- `Cargo.toml` `[workspace.dependencies]` already declares
  `engine-physics` (line 39); the crate gains real deps.
- ADR-051 gains three deviation entries: (a) SAP broad-phase vs
  BVH/SAH; (b) convex+primitive colliders only (no trimesh/heightfield);
  (c) no XPBD soft-body/cloth path (Verlet rope ships in 2D per
  ADR-086). None carry a future-phase tag — they are "engine does X,
  spec specifies Y, accepted."
- A new RNG-channel cohort (`physics.*`) is documented in the ADR-057
  amendment (Phase 7 PR 2).
- `tilemap` collision-rects (ADR-089) surface into the 2D module's
  static-collider set.

## Risks and tradeoffs

- **SAP degrades to O(n²) for n bodies sharing one axis interval.**
  Accepted at v0.5 scene scale; the `rigid_body_3d_100k` frame-pacing
  scene uses spatially-distributed bodies. Revisit when a scene
  demands BVH; the broad-phase trait boundary keeps the swap local.
- **PGS is iterative, not exact.** 8 iterations can leave residual
  penetration in tall stacks. Accepted — matches spec; warm-starting +
  Baumgarte keep it visually stable; the `box_stack_10` replay oracle
  pins the settled state.
- **Determinism is fragile.** Any future `f32` reduction that reorders
  by address breaks replay parity. Mitigated by the replay oracle
  running in CI and the FMA-off sim profile.

## Alternatives considered

- **AABB-BVH/SAH broad-phase (spec).** Rejected for v0.5 on the
  SAP-simplicity + determinism grounds above; recorded as the deviation
  to revisit, not a permanent design rejection.
- **XPBD unified solver (spec, for soft bodies).** Rejected for v0.5 —
  the rigid path uses sequential-impulse PGS which the spec also
  prescribes for rigid contact; XPBD soft/cloth is breadth the
  milestone does not need.
- **Position-based dynamics for rigid bodies.** Rejected — PGS is the
  spec's named solver and the books' subject.

## Verification

- `crates/engine-physics/tests/replay_parity.rs` — 1 000-step
  `box_stack_10`, `chain_settle`, `tilemap_collide_2d`; BLAKE3 digest
  of full world state must match across two runs on the same seed
  (mirrors ADR-033).
- Unit tests: GJK boolean against analytic sphere/box overlaps; EPA
  normal/depth against analytic penetration; SAP pair-set against a
  brute-force O(n²) reference; solver single-contact restitution and
  resting-contact convergence.
- `testbed/frame-pacing/scenes/rigid_body_3d_100k.ron` (Phase 7 PR 18)
  — frame-pacing budget row.
- `just ci` green at the PR-3 commit.
