# ADR-024 — Derive macros consolidated in engine-ecs-macro

- Status: Accepted
- Date: 2026-05-19 (expanded 2026-05-24 per audit §15 Phase-0 ADR sweep)
- Phase: 0 (pre-v1.0 — contract-exempt per risk R-03)
- Companion: ADR-001 (Rust as the implementation language),
  ADR-002 (hybrid ECS storage — `#[derive(Component)]`),
  ADR-031 (archetype storage — Reflect for component layout)

## Context

The engine needs two procedural derive macros in the foundation
layer:

- `#[derive(Component)]` — implements the ECS `Component` trait,
  with a `#[component(storage = "Table" | "SparseSet")]` attribute
  (spec IV.3 / ADR-002).
- `#[derive(Reflect)]` — implements the reflection `Reflect` trait
  (spec III.2, IV.7).

Rust requires procedural macros to live in a crate with
`proc-macro = true`, and such a crate can export *nothing else*.
The spec's Level 0 crate list (Part IV.1) names exactly one
proc-macro crate: `engine-ecs-macro`. The `Reflect` derive
logically belongs to `engine-reflect`, but `engine-reflect` is an
ordinary library crate and cannot host a proc-macro.

## Decision

`engine-ecs-macro` hosts **all** of the engine's derive macros,
not only the ECS ones. `#[derive(Reflect)]` lives there alongside
`#[derive(Component)]`. Any future derive (`#[derive(Asset)]`,
`#[derive(SaveMigrate)]`, etc.) goes here too.

Dependency policy:

- `engine-ecs-macro` pins specific versions of the
  procedural-macro toolchain: `proc-macro2`, `syn`, `quote`.
  These three crates are the engine's accepted proc-macro
  dependencies; no others are added without a new ADR.
- Version pinning matches what's reproducible in
  `Cargo.lock`; the audit's reproducibility cadence (ADR-052)
  catches silent updates.

Generated code policy:

- Generated code refers to its target traits by canonical
  absolute path (`::engine_core::ecs::Component`,
  `::engine_reflect::Reflect`), so a consumer crate must
  depend on `engine-core` / `engine-reflect` un-renamed.
- `engine-core` re-exports both the `Component` trait and the
  `Component` derive under one name, so a single
  `use engine_core::Component;` brings in both.

## Rationale

The alternative — adding an `engine-reflect-macro` crate —
introduces a crate the spec's authoritative crate list does not
contain, for no benefit beyond nominal tidiness. Consolidating
respects the spec's crate inventory and keeps proc-macro compile
cost in one place. The crate's doc comment states plainly that
it is the workspace's derive crate, so the slightly broad name
is not misleading.

The pinned proc-macro-toolchain deps (proc-macro2/syn/quote) are
the standard derive-macro stack. Pinning them explicitly
documents the allowlist (audit can grep "syn = " to verify only
the expected crate uses it).

The canonical-absolute-path discipline in generated code is the
proc-macro idiom for avoiding "consumer renamed engine-core"
brittleness. The cost (consumers must depend on un-renamed
engine-core) is trivial.

## Consequences

- `engine-ecs-macro` is a dependency of every crate that derives
  `Reflect`, even ones with no ECS involvement. The crate is
  tiny, so this is cheap.
- A future derive macro (e.g. `#[derive(Asset)]`) goes here too,
  by the same reasoning. If the macro count ever makes the name
  actively wrong, renaming the crate is a mechanical change —
  but that is not a Phase 0 concern.
- The proc-macro dependency allowlist is documented and
  enforceable; new proc-macro toolchain crates need an ADR
  amendment.
- The cross-crate trait-path discipline (every generated path is
  `::engine_*::...`) keeps the macro robust against consumer
  renames.

## Risks and tradeoffs

- **Crate name is slightly broad.** `engine-ecs-macro` hosts
  non-ECS macros. Mitigation: doc comment is explicit; a
  rename is a mechanical cost if it ever becomes load-bearing.
- **Proc-macro compile cost.** All derives share one compile;
  the crate's recompilation triggers many downstream
  re-derives. Mitigation: small, focused crate; incremental
  compilation absorbs the cost.
- **`syn` version churn.** `syn` 2.x → 3.x will require macro
  rewrites. Mitigation: pinning is explicit; the rewrite is a
  deliberate PR.
- **Path-rename brittleness.** If `engine-core` ever needs to
  re-export `Component` under a different module path, every
  derive must update. Mitigation: the re-export pattern is
  documented; the audit notices changes.

## Alternatives considered

- **`engine-reflect-macro`** as a separate crate. Cleaner
  domain partitioning; adds a crate not in the spec's
  inventory. Rejected per consolidation reasoning above.
- **Inline the `Reflect` derive in `engine-reflect`** via
  some non-proc-macro mechanism (e.g. `build.rs` codegen).
  Loses ergonomics; adds build complexity. Rejected.
- **Skip derive entirely**, require hand-written impls.
  Substantial boilerplate; defeats reflection's ergonomic
  win. Rejected.

## Verification

- `cargo test -p engine-ecs-macro` — derive output golden
  tests (the macro's output for a small fixture set is
  committed; regeneration is a reviewable PR).
- `cargo test -p engine-core` exercises `#[derive(Component)]`
  output indirectly through the component-storage tests.
- `cargo test -p engine-reflect` exercises
  `#[derive(Reflect)]` output indirectly through the reflection
  introspection tests.
- The `Cargo.lock` pins proc-macro2/syn/quote versions;
  reproducibility-build cadence (ADR-052) catches silent
  updates.
