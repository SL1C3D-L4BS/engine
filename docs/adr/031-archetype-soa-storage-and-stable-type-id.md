# ADR-031 — Archetype-SoA storage and stable type identity

- Status: accepted
- Date: 2026-05-20
- Phase: 3 — ENGINE CORE
- Supersedes: portions of ADR-002 (storage hybrid) — the hybrid is preserved,
  but Table components now live in archetype-grouped columns rather than
  per-type dense slot arrays.

## Context

Phase 1–2 shipped a flat per-component column ECS keyed by `std::any::TypeId`
(`crates/engine-core/src/ecs/world.rs` and `storage.rs`). It is correct and
deterministic, but the layout has two cliffs that the spec milestone names
(spec Part XXI, "1 000 000 entities, 60 FPS, on one core") makes
non-negotiable:

1. **Cache behaviour.** A query for `(Position, Velocity)` walks two
   independent `Vec<Option<T>>` arrays — random-access into the `Velocity`
   column for every live `Position`. At 1 M entities the load is dominated by
   L2/LLC misses on the cross-column fetch.

2. **Cross-architecture identity.** `TypeId` is process-private; its bit
   pattern is not stable across builds or architectures. The determinism
   contract (spec IV.2 / ADR-013) requires byte-equal frame digests across
   x86-64 and aarch64, so the per-frame hash key cannot use `TypeId`.

Bevy-style archetype-SoA solves both: entities sharing the same Table
component set live contiguously in one archetype, and a query for
`(Position, Velocity)` becomes a tight zipped iteration over two parallel
column arrays. The cross-arch identity story needs a parallel fix — a
hashed, deterministically-derived type id.

## Decision

Replace the flat per-component column layout with an **archetype index**:

- An [`Archetype`](../../crates/engine-core/src/ecs/archetype.rs) owns one
  [`AnyVec`] per Table component in its signature plus a flat
  `Vec<u32>` of entity indices. Rows are parallel across columns.
- [`ArchetypeSignature`] is a sorted, deduplicated `Vec<TypeStableId>` of
  Table components only. SparseSet components remain world-scoped sidecars
  per ADR-002 — the hybrid is preserved, the hybrid's boundary is moved.
- [`ArchetypeId`] is a dense `u32` allocated on first interning of a new
  signature. The signature → id table uses [`DeterministicHasher`]
  (ADR-028), so dense ids are reproducible across runs and architectures.
- Adjacency caches accelerate insert/remove:
  `(from, type_added) → to` and `(from, type_removed) → to`. Hot lookup so
  [`FastHasher`] is appropriate — the cache only affects insert performance,
  not the determinism digest.

Adopt a new cross-arch-stable type identifier:

- [`TypeStableId(pub u64)`](../../crates/engine-reflect/src/registry.rs)
  lives in `engine-reflect` so the engine and external scene loaders see the
  same definition.
- The `Component` derive (`crates/engine-ecs-macro/src/lib.rs`) computes the
  id at macro-expansion time as the first eight little-endian bytes of
  `BLAKE3(crate_name || "::" || ident)` and emits it as a literal `u64`. The
  trait gains `const STABLE_ID: TypeStableId`; the id is therefore available
  in `const` contexts at the cost of one BLAKE3 evaluation in the
  proc-macro.
- `crate_name` is `std::env::var("CARGO_CRATE_NAME")` at expansion. The
  full `module_path!()` qualifier is not visible to the proc-macro
  (`module_path!()` only resolves in source position), and forwarding it
  into the const would require const BLAKE3 (not available). The
  crate-name + ident pair is the closest stable qualifier that fits in a
  literal `const`.

Iteration order is fixed: queries walk matching archetypes in ascending
`ArchetypeId`, rows in ascending order. Because archetype ids are minted via
a deterministic-hashed insert order, the sequence is reproducible across
runs and architectures. This is the property the Phase 3 replay-parity
oracle (PR 3) will lean on.

## Consequences

### Positive

- The 1 M-entity benchmark (PR 3) can now meet its milestone: zipped
  parallel-column iteration over flat `*const T`/`*const U` pointers gives
  the L1d-friendly access pattern the spec demands.
- Determinism is preserved by construction: archetype interning is hashed
  with `DeterministicHasher`; query iteration sequences archetype then row
  in ascending order; the cross-arch frame digest is unaffected by the
  storage rewrite.
- Scene serialization gains a stable handle: `TypeStableId(0x…)` is
  meaningful in a hex dump and reversible via
  `TypeRegistry::register_stable_id` + `TypeRegistry::name_of`.

### Negative

- Insert/remove of a Table component on an entity now moves an entire row
  between archetypes (one `move_row_into` call per shared column). For
  churn-heavy components the cost is proportional to the entity's total
  component count. The escape hatch is `#[component(storage = "SparseSet")]`
  — ADR-002's two-backend hybrid is exactly what mitigates this.
- The `TypeStableId` derive uses `crate_name + ident`, not the full
  `module_path`. Two components with the same name in different modules of
  the same crate will collide. No in-tree component shares a name; if a
  future component does, the macro will need to gain a manual
  `#[component(stable_path = "…")]` opt-out (deferred).
- The determinism golden bumps once. The plan documents this as the
  Phase 1 / Phase 2 regenerate-once-on-intent pattern. New golden:
  `a87a584279a2a06a` (`engine-core/tests/golden-core.txt`).
- `engine-ecs-macro` gains a `blake3` build-time dependency. The
  dependency is proc-macro-only (compile-time), so it does not enter the
  engine's runtime dependency graph; ADR-025 already audits BLAKE3.

### Neutral

- `std::any::TypeId` is grandfathered for the world's resource map only —
  resources are not iterated by the determinism oracle, and the CI guard
  recognises the `// allow: resources` comment on the relevant lines. The
  rest of the ECS source is grep-prohibited from naming `TypeId::of::<`.

## Sequencing

This is PR 1 of Phase 3. PR 2 (ADR-032) builds the fiber job system on
top of `engine-platform`; PR 3 (ADR-033) wires the two together with the
deterministic parallel scheduler, the 1 M-entity benchmark, and the
replay-parity oracle.

## Files changed

- `crates/engine-core/src/ecs/archetype.rs` (new) — `Archetype`,
  `ArchetypeId`, `ArchetypeSignature`, `AnyVec`, `ArchetypeIndex`,
  adjacency caches.
- `crates/engine-core/src/ecs/query.rs` (new) — `WorldQuery` trait and
  iterator over matching archetypes; impls for `&T`, `Mut<T>`, `(&A, &B)`.
- `crates/engine-core/src/ecs/type_id.rs` (new) — re-export of
  `TypeStableId` plus the `stable_id_of::<T>()` const helper.
- `crates/engine-core/src/ecs/world.rs` — rewritten to route Table
  components through archetypes and keep SparseSet components world-side.
- `crates/engine-core/src/ecs/storage.rs` — `DenseColumn` retired;
  `SparseColumn` retained as the SparseSet backend.
- `crates/engine-core/src/ecs/entity.rs` — `EntityAllocator` now tracks an
  `EntityLocation { archetype, row }` per slot.
- `crates/engine-core/src/ecs/mod.rs`, `crates/engine-core/src/lib.rs` —
  re-exports of `ArchetypeId`, `ArchetypeSignature`, `Query`,
  `TypeStableId`.
- `crates/engine-reflect/src/registry.rs` — `TypeStableId(u64)`;
  back-lookup map `BTreeMap<TypeStableId, &'static str>`.
- `crates/engine-ecs-macro/src/lib.rs`,
  `crates/engine-ecs-macro/Cargo.toml` — `Component` derive emits
  `STABLE_ID`; `blake3` added as a proc-macro build-time dep.
- `crates/engine-core/tests/archetype.rs` (new) — adjacency, both
  backends, swap-remove correctness, joint queries.
- `crates/engine-core/tests/determinism.rs`,
  `crates/engine-core/tests/golden-core.txt` — extended with the
  64-archetype sweep; golden regenerated once.
- `.github/workflows/ci.yml` — new guard rejecting `TypeId::of::<` inside
  `crates/engine-core/src/ecs/`, with the resource-map allowlist comment.
- `justfile` — `archetype-baseline` recipe.
- `engine.toml` — `phase = "3"`; comment updated.
- `docs/observatory/archetype-baseline.md` (new) — baseline placeholder
  (no CI gate).
