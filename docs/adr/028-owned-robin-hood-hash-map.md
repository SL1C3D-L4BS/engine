# ADR-028 — Owned Robin Hood hash map

- **Status**: accepted
- **Phase**: 2 (Linux Systems, spec Part XXI)
- **Date**: 2026-05-19

## Context

The engine had been using `std::collections::HashMap` (SwissTable, SipHash by
default) in three hot paths:

- `engine_core::ecs::World` — both the per-archetype component column
  lookup (`HashMap<TypeId, Box<dyn AnyColumn>>`) and the resource lookup
  (`HashMap<TypeId, Box<dyn Any>>`).
- `engine_asset::ContentStore::blobs` — content-addressed blob store.
- `engine_asset::AssetServer::cache` — name-keyed handle cache.

Phase 2 is the "own the platform surface" phase (spec Part XXI). Three things
push for an owned implementation, none satisfied by std's map alone:

1. **Tail-latency control.** SwissTable's metadata-byte probing has good
   *average* throughput, but its worst-case probe distance is workload- and
   hasher-dependent. Robin Hood with backward-shift deletion bounds the
   *variance* of the probe distance: every entry is at most `dib_max` slots
   from its initial bucket, and `dib_max` is observably small under load
   factors below 7/8. Inner ECS loops will want this once the archetype
   rewrite (Phase 3) starts touching thousands of components per frame.
2. **Cross-architecture determinism.** PR 2's mmap'd pak loader and the
   foundation determinism contract (ADR-013) need a probe sequence that
   does not differ between x86-64 and aarch64. SwissTable's RandomState
   hasher is fundamentally non-deterministic, and even fxhash on std's
   table varies because the hash-byte fold differs from what we feed
   downstream. An owned table with an owned BLAKE3-keyed hasher closes
   that gap exactly.
3. **R-02 substrate.** The engine's foundation layer is owned in-tree (see
   [foundation-layer-deviations] in auto-memory: engine-telemetry and
   engine-i18n already ship without their planned third-party deps). The
   hash map is a more load-bearing substrate than either of those, and
   leaving it as std-only contradicts the policy.

## Decision

Ship `engine_core::collections::HashMap` with:

- **Open addressing, Robin Hood probing, backward-shift deletion.**
  Power-of-two capacities; load-factor cap 7/8; double on grow. Slots store
  a 32-bit truncated hash and a 16-bit DIB (Distance from Initial Bucket).
- **Two hashers**, both owned:
  - `FastHasher` — multiplicative FxHash-style (constant
    `0x517c_c1b7_2722_0a95`). For well-distributed keys (TypeId, content
    hashes). Default hasher for the type alias.
  - `DeterministicHasher` — BLAKE3 keyed with a fixed 32-byte key
    (`engine-core-deterministic-hasher`). Every `write_uN` override
    serializes operands to little-endian, so the byte stream into BLAKE3
    is cross-architecture stable. Use whenever the iteration or probe
    sequence could leak into a cross-arch invariant.
- **API parity surface** with `std::collections::HashMap`: `new`,
  `with_capacity`, `with_hasher`, `insert`, `get`, `get_mut`, `remove`,
  `contains_key`, `len`, `is_empty`, `capacity`, `clear`, `iter`,
  `iter_mut`, `keys`, `values`, `values_mut`, `drain`, `retain`. Plus
  `probe_distance_histogram()`, used only by the parity oracle.

### Migration sites

- `World::columns` — `FastHasher`. Keys are `TypeId`; only call shape is
  point lookup; no system iterates the column map by key.
- `World::resources` — `DeterministicHasher`. Insurance against a future
  system that iterates resources and feeds the order into the frame
  digest. Today no system does — the digest stays unchanged, verified by
  re-running `just determinism`.
- `ContentStore::blobs` — `FastHasher`. Keys are 256-bit content hashes;
  already random-looking.
- `AssetServer::cache` — `FastHasher`. Keys are string asset names.

### What is *not* promised

**Stable iteration order**, even with `DeterministicHasher`. Probe
positions are a function of capacity; growing the table reorders every
entry. Callers that need order must use `std::collections::BTreeMap`
instead. This is stated in the crate-level docs and is repeated here so a
future PR cannot accidentally rely on it.

## Consequences

- Tail-latency under load is governed by Robin Hood's bounded probe
  variance instead of SwissTable's worst-case scan-cluster behaviour. The
  parity oracle (`tests/collections_parity.rs`) pins both the semantic
  behaviour (against `StdHashMap`) and the algorithmic shape (against an
  in-test naive Robin Hood reference, with a committed
  probe-distance-histogram golden).
- One additional substrate is owned by the engine. CI gates this in two
  ways: (a) a workspace-wide grep rejecting `std::collections::HashMap`
  and bare `use std::collections::HashMap` outside `tests/` and
  `benches/`, and (b) the existing cross-arch determinism oracle, which
  re-runs after the migration to confirm the ECS resource map's switch
  to `DeterministicHasher` did not perturb the committed digest.
- One ergonomic regression: we do not ship an `Entry` API. The two
  pre-existing `Entry`-shaped call sites (`ContentStore::insert`) were
  rewritten as `contains_key + insert`. Future code should follow the
  same pattern.

## References

- Pedro Celis (1986), *"Robin Hood Hashing"* — original probe-distance
  equalization proof.
- Emmanuel Goossaert, *"Robin Hood hashing: backward shift deletion"*
  (2013) — the deletion scheme we adopt.
- TLPI Ch. 7 §7.5 — segregated free lists; the size-class intuition
  carries over to the bucket allocation policy.
- ADR-013 — Determinism Contract (why a cross-arch-stable hash matters).
- ADR-026 — General free-list arena (the previous "own the substrate"
  decision; this ADR follows the same pattern for a different data type).

[foundation-layer-deviations]: /home/doodlebob/.claude/projects/-home-doodlebob-Projects-engine/memory/foundation-layer-deviations.md
