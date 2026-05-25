# ADR-014 — Hot/cold component separation

- Status: Accepted
- Date: 2026-05-18 (expanded 2026-05-24 per audit §15 Phase-0 ADR sweep)
- Phase: 0 (pre-v1.0 — contract-exempt per risk R-03)
- Companion: ADR-002 (hybrid ECS storage), ADR-031 (archetype-SoA
  storage), Phase 10 (memory debugger that surfaces violations)

## Context

The engine's 1 M-entity portfolio target (spec §IV.3 / ADR-033)
puts hot-path iteration on the cache hierarchy's main constraint.
At 1 M entities × 16 bytes per component × one hot system per
frame, the loaded working set is 16 MiB — already past the L3
on most consumer CPUs. Every byte that *isn't* needed by the
hot system but lives in the same archetype is cache pollution.

The classical ECS pitfall: a single component struct with hot
fields (position, velocity) and cold fields (name, description,
ID, audit-trail data). Every hot iteration loads the cold fields
through cache lines, paying the bandwidth cost for data the
system doesn't touch.

The literature (`DataOrientedDesign.pdf`) names the pattern:
*hot/cold split*. Hot fields live in one component (small,
SoA-friendly); cold fields live in a separate component
(referenced by entity ID, queried only when needed).

## Decision

The engine's component design discipline:

- **Hot components** contain only fields touched per frame by the
  inner-loop systems. Typical size: 8–24 bytes. Examples:
  `Position { x, y, z, _padding }`, `Velocity { x, y, z, _padding }`,
  `Health { current, max }`.
- **Cold components** contain everything else: names, descriptions,
  scripted-event metadata, audit-trail data. Referenced separately
  by the editor or queried by cold systems (UI, save/load).
- **Discipline is convention, not enforcement.** The engine does
  not statically prevent a hot+cold mix in one component; the
  diagnostic mechanism is the memory debugger (Phase 10), which
  surfaces "system Foo iterates archetype X and touches Y bytes
  but only reads Z bytes — Y/Z=4.2× cache pollution."
- **SparseSet storage (ADR-002)** is the alternative for change-
  tracked components; SparseSet's per-entity indirection is
  itself a form of cold-storage indirection.

## Rationale

The 1 M-entity target is a hard ceiling on per-iteration cost.
A 16 MiB inner-loop working set exceeds L3 on most consumer CPUs;
an 80 MiB working set (5× cold-field pollution) is bandwidth-
bound on DDR4-3200, simply not achievable at 60 FPS.

Discipline-by-convention rather than enforcement-by-language
reflects the engine's R-02 stance: the *measurement* (memory
debugger) is owned; the *enforcement* (which fields are hot
vs. cold) is the author's judgment, informed by the
measurement.

The pattern is well-trodden: Unity's DOTS, Bevy's component
storage, Flecs's pre-allocated archetypes all share the same
hot/cold convention with the same measurement-driven enforcement.

## Consequences

- The engine ships a memory-debugger tool (Phase 10,
  `engine-memdbg` per spec §XVI) that profiles archetype
  iteration and surfaces hot/cold violations.
- Author-facing documentation in `docs/architecture/engine-core.md`
  (and the future `docs/onboarding/`) calls out the
  hot/cold pattern.
- The default storage for `#[derive(Component)]` is Table
  (ADR-002); table storage is the case where hot/cold
  separation matters most.
- The `#[component]` attribute on the derive macro could
  someday gain a `#[component(hot, cold)]` annotation that
  the memory debugger could use as a contract; not Phase 0
  scope.

## Risks and tradeoffs

- **Convention drifts.** Without enforcement, authors can mix
  hot and cold fields. Mitigation: the Phase-10 memory
  debugger's diagnostic visibility makes the cost visible at
  development time.
- **Per-entity indirection cost** for cold-component access.
  A cold lookup ("what is this entity's name?") goes through
  an archetype query or a sparse-set fetch — slower than a
  direct field access. Acceptable: cold queries are rare.
- **Component proliferation.** Splitting a single struct into
  multiple components increases the engine's component
  inventory. Pattern: name them in pairs (e.g. `Transform` /
  `TransformLabel`) so the relationship is documented in code.

## Alternatives considered

- **Enforce hot/cold split at the type system level** (`#[hot]`
  / `#[cold]` attribute that the derive checks). Considered;
  rejected for Phase 0 because the heuristic for what's hot
  is workload-dependent and the static analysis is hard.
- **Cold fields in a separate `Resource` keyed by Entity.**
  Equivalent to a sparse-set component. Acceptable pattern;
  no new mechanism needed (the sparse-set storage already
  exists).
- **Component AoS within archetypes** (a per-archetype struct
  with all fields). What classical ECS engines did before
  SoA; defeats the cache argument. Rejected.

## Verification

- The memory debugger (Phase 10) surfaces hot/cold cache
  pollution as a diagnostic.
- The million-entity benchmark
  (`cargo bench -p engine-core --bench million_entities`)
  measures inner-loop throughput; a hot/cold violation in
  the benchmark's components would show up as a regression.
- Author documentation (Phase 10 onboarding writeup) covers
  the hot/cold convention; the convention's adoption is the
  workload-dependent verification.
- A Phase 10+ lint (clippy-style) could surface "this
  component is >32 bytes; consider hot/cold split." Tracked
  for the memdbg work, not Phase 0.
