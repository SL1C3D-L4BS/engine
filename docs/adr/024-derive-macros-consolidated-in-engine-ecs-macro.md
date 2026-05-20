# ADR-024 · Derive macros consolidated in engine-ecs-macro

- Status: Accepted
- Date: 2026-05-19
- Phase: 0 (pre-v1.0 — contract-exempt per risk R-03)

## Context

The engine needs two procedural derive macros in the foundation layer:

- `#[derive(Component)]` — implements the ECS `Component` trait, with a
  `#[component(storage = "Table" | "SparseSet")]` attribute (spec IV.3 /
  ADR-002).
- `#[derive(Reflect)]` — implements the reflection `Reflect` trait
  (spec III.2, IV.7).

Rust requires procedural macros to live in a crate with `proc-macro = true`,
and such a crate can export *nothing else*. The spec's Level 0 crate list
(Part IV.1) names exactly one proc-macro crate: `engine-ecs-macro`. The
`Reflect` derive logically belongs to `engine-reflect`, but `engine-reflect`
is an ordinary library crate and cannot host a proc-macro.

## Decision

`engine-ecs-macro` hosts **all** of the engine's derive macros, not only the
ECS ones. `#[derive(Reflect)]` lives there alongside `#[derive(Component)]`.

The generated code refers to its target traits by canonical absolute path
(`::engine_core::ecs::Component`, `::engine_reflect::Reflect`), so a consumer
crate must depend on `engine-core` / `engine-reflect` un-renamed. `engine-core`
re-exports both the `Component` trait and the `Component` derive under one
name, so a single `use engine_core::Component;` brings in both.

## Rationale

The alternative — adding an `engine-reflect-macro` crate — introduces a crate
the spec's authoritative crate list does not contain, for no benefit beyond
nominal tidiness. Consolidating respects the spec's crate inventory and keeps
proc-macro compile cost in one place. The crate's doc comment states plainly
that it is the workspace's derive crate, so the slightly broad name is not
misleading.

## Consequences

- `engine-ecs-macro` is a dependency of every crate that derives `Reflect`,
  even ones with no ECS involvement. The crate is tiny, so this is cheap.
- A future derive macro (e.g. `#[derive(Asset)]`) goes here too, by the same
  reasoning. If the macro count ever makes the name actively wrong, renaming
  the crate is a mechanical change — but that is not a Phase 0 concern.
