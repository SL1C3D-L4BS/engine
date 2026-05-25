# ADR-002 — Hybrid ECS storage (archetype-SoA + opt-in SparseSet)

- Status: Accepted
- Date: 2026-05-18 (expanded 2026-05-24 per audit §15 Phase-0 ADR sweep)
- Phase: 0 (pre-v1.0 — contract-exempt per risk R-03)
- Companion: ADR-031 (archetype-SoA storage and stable TypeId — the
  concrete Phase-3 implementation), ADR-014 (hot/cold component
  separation), ADR-024 (derive macros)

## Context

The engine's portfolio target is 1 M entities at 60 FPS on a single
core (spec §IV.3; realised in Phase 3 per ADR-033). The data layout
that the simulation systems iterate is the single largest determinant
of whether that target is met. The literature
(`DataOrientedDesign.pdf`, `EntityComponentDesignSystemPatterns.pdf`
in the project's reference library) and the Bevy 0.18 architecture
review converge on the same conclusion: archetype-SoA is correct for
*hot iteration* (every motion / physics / culling system), sparse
storage is correct for *change-tracking and tag-like* components
(rare modifications, frequent membership queries).

Pure-archetype engines (Flecs, EnTT in its archetype mode) pay
significant migration cost when a single component is added or
removed from an entity (every component column copies between
archetypes). Pure-sparse engines (older EnTT, Bevy's
SparseSet-only mode) pay a per-component indirection on every
hot-path access. A hybrid model lets the user choose per component.

## Decision

The default storage for a `#[derive(Component)]` type is
**Table** (archetype-SoA). The author opts into sparse storage
explicitly:

```rust
#[derive(Component)]
#[component(storage = "SparseSet")]
struct DamageTaken { points: f32, frame: u32 }
```

Storage selection is a per-type compile-time decision encoded in
the `Component` trait's `STORAGE: StorageKind` associated const.
The ECS runtime branches on this const when inserting / removing /
querying.

`engine-ecs-macro` (ADR-024) parses the `#[component(storage = "...")]`
attribute. The Phase-3 implementation (ADR-031) realises both
backends; the rest of the engine is unaware of which backend a
component uses (queries iterate identically).

## Rationale

Bevy 0.18's mature codebase served as the validation: identical
benchmarks under Bevy's hybrid mode beat pure-archetype on
add/remove-heavy workloads (status effects, transient damage,
input events) and beat pure-sparse on read-heavy workloads
(transform hierarchies, render queues, physics integration). The
engine's portfolio target needs both characteristics.

The compile-time selection (vs. a runtime config) keeps the hot
path branchless: the table-storage query never branches on
"maybe this component is sparse"; the sparse-storage query
never branches on "maybe this component is in a table." The
const is also visible to the scheduler's R/W declarations
(ADR-033).

The default-to-table choice is deliberate. The portfolio is
dominated by hot-iteration systems; defaulting to sparse would
trade the wrong way. Authors who *need* sparse semantics
(change-tracking, tag membership) consciously opt in.

## Consequences

- Two backend implementations to maintain (`ArchetypeStorage` and
  `SparseSetStorage` in `engine-core::ecs`). The Phase-3 work
  (ADR-031) implemented both; both are part of the determinism
  oracle's frame-digest scope.
- A component's storage choice is a compile-time decision; changing
  it is a breaking change for any system that depends on
  membership-stability semantics (a Table component reuses its
  archetype across archetype moves; a SparseSet component does
  not).
- The `#[derive(Component)]` macro is non-trivial. The `Reflect`
  macro (ADR-024) lives alongside it for the same proc-macro-cost
  reason.
- Change detection (`Changed<T>` filter) is cheaper on SparseSet
  (per-entity change bit), more expensive on Table (per-archetype
  generation tag); the choice is visible to the query author.

## Risks and tradeoffs

- **Two storage paths means twice the surface for storage bugs.**
  Mitigation: the determinism oracle digests both storage classes;
  any divergence between backend behaviours is caught at the
  cross-architecture determinism gate.
- **Migration cost on archetype moves.** A common pitfall in
  archetype engines: an entity that gains/loses a component per
  frame triggers a full archetype rewrite. The engine documents
  the rule (per-frame-changing components should be SparseSet)
  in the `Component` derive's doc comment; the memory debugger
  (Phase 10) will surface violations.
- **Author confusion** about which storage to pick. Mitigation:
  the `engine-tui` introspector (Phase 10) surfaces "your
  Damage component sees 14 ms/frame of archetype churn — try
  SparseSet" as a diagnostic.

## Alternatives considered

- **Pure archetype** (Flecs model). Faster on the hot path; slower
  on the (frequent) add/remove path. Rejected: the portfolio
  characteristics demand both.
- **Pure SparseSet** (older EnTT). Faster on add/remove; slower on
  hot iteration. Rejected: 1 M-entity hot iteration is the
  portfolio's headline target.
- **Per-component runtime selection** (a `Box<dyn Storage>` per
  type). Lost the const-propagation that keeps the queries
  branchless. Rejected.
- **Bevy as a vendored dependency.** Discussed and rejected per
  ADR-031's reasoning: the engine owns the storage to own the
  determinism contract.

## Verification

- `cargo test -p engine-core` — both storage backends pass the
  same query-iteration corpus.
- `cargo test -p engine-core --test determinism` — frame digests
  identical across both backends and across architectures.
- `cargo bench -p engine-core --bench million_entities` —
  Table-backed components clear the 1 M-entity / 60 FPS target;
  see `docs/observatory/million-entities-baseline.md`. SparseSet-
  backed components have a parallel bench tracked there.
- The replay-parity oracle (`crates/engine-core/tests/replay_parity.rs`,
  ADR-033) exercises systems whose R/W sets include both Table
  and SparseSet components.
