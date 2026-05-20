# ADR-033 — Parallel deterministic scheduler

- Status: accepted
- Date: 2026-05-20
- Phase: 3 — ENGINE CORE
- Companion: ADR-031 (archetype storage), ADR-032 (fiber job system)

## Context

Phase 3's portfolio target is 1 M entities at 60 FPS on one core, plus
deterministic frame digests across architectures and worker counts. PR 1
gave us the data layout (archetype-SoA), PR 2 gave us the compute
substrate (owned thread pool + JobGraph). PR 3 is the wiring: a
`Schedule` that dispatches non-conflicting systems through `JobGraph` and
still produces frame-by-frame world state identical to a sequential run.

The spec puts two demands on this layer at the same time:

- **Spec IV.3** — "non-conflicting systems run in parallel".
- **Spec IV.2 / ADR-013** — "byte-identical frame digests across runs,
  architectures, and worker counts".

Those demands meet here. Any divergence — a system reading a component
it didn't declare, two parallel jobs racing on a shared field — would
violate the determinism contract on the very gate Phase 9 netcode
depends on.

## Decision

Three coordinated additions to `engine-core::ecs::Schedule`:

### 1. R/W-declared system registration

```rust
schedule.add_system_with_access(
    Phase::Update,
    "motion",
    &[Velocity::STABLE_ID],   // reads
    &[Position::STABLE_ID],   // writes
    |w: &mut World| { /* ... */ },
);
```

R/W sets are explicit lists of `TypeStableId` (ADR-031). The plan
considered a query-DSL `SystemParam` macro that would derive R/W from
the function signature; deferred to Phase 4+ because it ties the
parallel scheduler to a richer query-DSL implementation than the one
Phase 3 ships. The replay-parity oracle is the runtime backstop against
declarations that lie — see §3 below.

The legacy `add_system(phase, name, fn)` is kept for "exclusive"
systems (system that take `&mut World` and might do anything). Any
phase containing an exclusive system falls back to sequential execution
in `run_on`; the rest of the frame still runs in parallel.

### 2. Per-phase JobGraph dispatch

`Schedule::run_on(&mut self, world, pool: &ThreadPool)` iterates
`Phase::ALL` in order. For each phase:

1. Collect the systems registered in that phase, in registration order.
2. If any are exclusive, run them sequentially — same observable order
   as `Schedule::run`.
3. Otherwise build a `JobGraph` (ADR-032), one job per system, with
   `JobGraph::add_job(reads, writes, body)` consuming the declared sets.
4. `graph.run_on(pool)`.

Phases never overlap. Within a phase, JobGraph's conflict rule
(write/write and write/read pairs serialise; read/read pairs commute,
ADR-032) sequences any pair that touches a shared `TypeStableId`. The
ordering of *parallel-safe* pairs is non-deterministic across runs —
but because parallel-safe means R/W-disjoint, no parallel pair can
observe the other's writes, so the final world state is identical.

The `&mut World` reborrow in worker closures is `unsafe`; safety
discipline lives in the `dispatch_phase` SAFETY block:

- Two non-conflicting systems never touch the same archetype column
  (Table or Sparse), the same sparse row, or the same resource slot.
- Structural mutation (entity spawn/despawn, resource insert/remove)
  is reserved for exclusive systems — those run sequentially.

### 3. Replay-parity oracle (`tests/replay_parity.rs`)

The runtime backstop. A 1000-entity workload with six declared-access
systems runs 100 frames, snapshotting the BLAKE3 digest of the
world's component arrays at frames 1, 10, 100. The same workload is
re-run via `Schedule::run_on` at worker counts `{1, 2, 4,
available_parallelism()}` and every snapshot must match the
single-threaded reference. Wired into the CI determinism job so it
runs on both x86-64 and aarch64.

If a system declares it only writes `Position` but actually writes
`Velocity`, two parallel jobs can race and the digest diverges — the
oracle fails immediately, naming the worker count that broke.

## Consequences

- The scheduler is *static* — system R/W sets are fixed at registration.
  Dynamic per-frame conditional access is a Phase 4+ feature.
- The 1 M-entity / 60 FPS milestone gate is measured by
  `cargo bench -p engine-core --bench million_entities`, captured in
  `docs/observatory/million-entities-baseline.md`. Not a CI gate —
  runner noise makes a hard threshold infeasible in shared CI.
- The `unsafe` reborrow in `dispatch_phase` is the only `unsafe` block
  in the engine-core source. It is justified entirely by the R/W
  declaration contract; the replay-parity oracle is the verifier.
- `Schedule` system closures must now be `Send` (they cross thread
  boundaries through the pool). All existing test systems already
  satisfy this; the breaking-change surface is "no `Rc`/`Cell` in
  systems," which matches the spec's determinism stance anyway.

## Risks and tradeoffs

- **Lying declarations.** A system that writes through a `Resource` that
  aliases component data could escape the conflict analysis. Mitigated
  by: (1) the resource map is keyed by `TypeId` and disjoint from
  components; (2) the oracle would catch any architectural alias because
  the digest folds both component and resource state.
- **Collect-then-mutate pattern.** The PR 1 query DSL doesn't yet
  support `(Mut<A>, &B)` joins, so the oracle workload and the
  million-entities bench both use `for_each` collect-into-Vec +
  `get_mut`. That allocates one Vec per frame per join system — 24 MiB
  on a 1M Vec3-pair traversal. The Phase 4 query DSL upgrade removes
  the allocation; the bench number recorded today is the upper bound
  of the regression that upgrade will close.
- **No dynamic load-balancing knob.** JobGraph dispatches one job per
  system; if a single system dominates a phase, parallelism collapses.
  The fix is intra-system parallel-for (Phase 4+ feature), not a
  scheduler change.

## Alternatives considered

- **Bevy-style stage/SystemSet groups with implicit ordering.** Defers
  conflict to runtime borrow-checking on a per-component basis. We
  rejected it for the same reason ADR-025 rejected vendored runtimes:
  the implicit machinery makes the determinism failure mode harder to
  oracle-test.
- **Compile-time R/W extraction via the `SystemParam` macro.** The
  ergonomic win is real (no manual `&[Velocity::STABLE_ID]`), but the
  macro requires a query DSL we don't ship yet. Deferred to Phase 4+
  per plan.
- **Fiber-per-system dispatch (use the ADR-032 fibers here directly).**
  Phase 3 keeps fibers as an exported primitive but dispatches through
  the thread pool's job queue, not through cooperative yields. Fibers
  enter the scheduler when a system needs to await a GPU fence or a
  network event — Phase 5+ territory.

## Verification

- `cargo test -p engine-core` — all unit tests pass, plus
  `tests/replay_parity.rs` and the existing `tests/determinism.rs`
  (golden hash unchanged from PR 1).
- `cargo test -p engine-core --test replay_parity` — runs the
  cross-worker-count parity check (~0.5 s).
- `cargo bench -p engine-core --bench million_entities` — records
  per-frame wall-clock at 10k / 100k / 1M entities, sequential and
  parallel. Numbers in `docs/observatory/million-entities-baseline.md`.
- CI determinism job runs the replay-parity oracle on both x86-64 and
  aarch64.
- `cargo clippy --workspace --all-targets -- -D warnings` green.

## Addendum (2026-05-20) — Engine Core v0.1.1: milestone closed

The "Risks and tradeoffs" section called out a *collect-then-mutate*
allocation pattern as the cause of the 1M sequential frame landing at
~33 ms (over the 16.6 ms milestone target). A v0.1.1 follow-up added
the three named-as-deferred joint `WorldQuery` impls and rewrote the
bench plus the three replay-parity systems that needed them:

- `(Mut<A>, &B)`, `(&A, Mut<B>)`, `(Mut<A>, Mut<B>)` — each a
  structural clone of the existing `(&A, &B)` impl with `*mut` swapped
  in for the appropriate slot. Safety: A and B are distinct generic
  types (different `TypeStableId`), so they live in different `AnyVec`
  allocations within the same archetype. Two simultaneous reborrows
  from disjoint allocations cannot alias. A `debug_assert_ne!` in
  `build_arch_state` traps the `A == B` foot-gun at the cost of one
  debug-only branch (release builds stay branchless on the
  per-archetype hot path).
- The bench's `motion` system collapsed from a 24 MiB-per-frame Vec
  collect-then-mutate to a single archetype-stream walk. 1M sequential
  median: **33 ms → 4.35 ms** (7.6× speedup; cleanly under the
  16.6 ms milestone gate).
- The replay-parity oracle (the runtime backstop for the new
  `unsafe` code) was rewritten to drive `motion`, `bounce`, and `cast`
  through the new joint queries — and stayed green at all worker
  counts. The determinism golden is unchanged: the new impls are
  additive, the existing `(&A, &B)` path is untouched.

Numbers and the full history live in
`docs/observatory/million-entities-baseline.md`. The "richer DSL is
Phase 4+" wording elsewhere in this ADR refers only to the n > 2 tuple
case, filters (`With<T>` / `Without<T>`), and the `SystemParam`-style
macro — all still deferred.
