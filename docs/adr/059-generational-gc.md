# ADR-059 — Generational GC for the sli VM

- Status: Accepted (implementation lands with Phase-0 catchup PR alongside
  this ADR)
- Date: 2026-05-24
- Phase: 0 (foundation closure; declared in the Phase-0 catchup PR,
  formalises a Phase-4 design commitment that was deferred for slicing)
- Companion: ADR-007 (owned scripting VM — the parent), ADR-035 (sli
  register VM + GC — the single-generation design this supersedes the
  collector half of), ADR-060 (aggregate opcodes — supplies the
  mutating opcodes the write barrier fires from), ADR-013 (determinism
  contract — GC pause is on the per-frame budget), spec §IV.7

## Context

ADR-035 specified a tri-color mark-and-sweep heap with a generational
variant as the design end-state. The Phase-4 implementation
(`crates/engine-script/src/gc/`) shipped the architecture as a
*single-generation* tri-color heap: every allocation lives in one
`Vec<Slot>`, every collection is a major collection over the entire
heap, and the four submodule files (`nursery.rs`, `old_gen.rs`,
`remembered.rs`, `barrier.rs`) held ~10 lines of placeholder types
reserving the layout for the follow-up.

That follow-up is this ADR. The spec's §IV.7 budget — sub-millisecond
GC pauses on the simulation thread, with a 250 µs p99 target — is not
achievable with the single-generation collector once the heap grows
beyond a few thousand live objects: the mark walk alone exceeds the
budget. The classical generational hypothesis (Jones/Lins,
Wilson 1992: "most objects die young") is the design lever. Promoting
long-lived objects out of the high-frequency mark cycle is the
established way to keep the pause headline within budget.

The architectural pieces all have to land together because the
generational discipline is the product of their interaction:

1. **Nursery (young generation).** A bump-allocator over a soft-capped
   slot count. Every allocation begins here. Minor collections walk
   *only* the nursery.
2. **Old generation.** Mark-and-sweep over the promoted survivors.
   Touched only by major collections.
3. **Remembered set.** Card-marking record of old→young edges
   populated by the write barrier and consumed by minor collections
   as additional roots. Without it, a minor collection cannot tell
   which old-gen objects hold pointers into the nursery and would
   either prematurely free still-reachable young objects (correctness
   bug) or scan all of old-gen (defeats the generational win).
4. **Write barrier.** Fires on mutating opcodes (ADR-060). Records
   the card containing the source's slot and the discrete edge into
   the remembered set.

## Decision

The sli VM's heap is generational, with the four-pillar architecture
above. The Phase-0 catchup PR substantively implements each pillar and
refactors the `Heap` façade in `crates/engine-script/src/gc/mod.rs` to
route allocation and access through the nursery and old generation.

### Handle layout

`GcHandle` is a `u32`:

- Bit 31 (`OLD_GEN_BIT = 0x8000_0000`): 0 = nursery, 1 = old gen.
- Bits 0–30 (`INDEX_MASK = 0x7FFF_FFFF`): per-generation slot index.

The encoding lets the dispatcher and the write barrier classify a
handle in one instruction (a bit test) without a heap lookup. Handles
remain stable across collections **except** at promotion time, when a
nursery handle becomes an old-gen handle. The dispatcher receives a
remap table (`Vec<(GcHandle, GcHandle)>`) from `minor_collect` so any
external `Value` referencing a promoted slot can be rewritten.

### Collection policy

- **Allocation** → always into the nursery; `Heap::alloc` returns a
  nursery handle.
- **Minor collection** (`Heap::minor_collect`) → triggers when
  allocations since last GC cross `collect_after_allocations` (default
  4096) or when the nursery crosses `NURSERY_SOFT_CAP` (65 536 slots).
  Walks roots + the remembered set's recorded sources. Survivors with
  age ≥ 2 are promoted to old gen.
- **Major collection** (`Heap::collect`) → walks *both* generations
  from roots. Preserves the pre-ADR-059 oracle semantics: existing
  `tests/gc_oracle.rs` tests call `collect` and expect a full
  mark-and-sweep. Handles remain stable across `collect()` (no
  promotion happens during a major).

### Card size

512 bytes per card (SpiderMonkey-derived sweet spot for engines this
size). The card byte array sizes itself to `old_gen.capacity()` on
each write-barrier fire. A typical workload (no old→young edges) has
zero dirty cards and the minor collector skips the remembered scan in
~one `Vec::iter` pass.

### Promotion policy

A nursery object is promoted on the collection where its survival
counter reaches `PROMOTION_AGE = 2`. Two means "survived two minor
collections." Aggressive promotion (PROMOTION_AGE = 1) was considered
and rejected — it makes the old gen grow faster than necessary and
defeats the generational hypothesis for the common case of objects
that live for ~one tick.

## Rationale

The generational hypothesis is the entire reason this ADR exists. The
nursery is small (4 MiB / 64 K slots soft cap); the minor collector
visits only that. The old gen is touched by major collection only.
The remembered set keeps the minor collector's working set bounded by
the count of dirty cards × CARD_SIZE.

Three properties of the chosen layout:

1. **Single u32 handle.** The high-bit encoding keeps `GcHandle`
   `Copy`-trivial; the dispatcher's `Value` enum already wraps
   `GcHandle` for its four GC-backed variants. No allocation overhead,
   no per-handle metadata table.
2. **Card marking instead of a per-edge log.** A per-edge log would
   grow without bound under hot-loop mutation; cards collapse N
   stores within the same 512 B region into one byte. The cost is
   that the minor collector must scan the card's slots — acceptable
   because dirty cards are rare in the normal case.
3. **Promotion via remap table, not handle indirection.** The
   alternative (a stable indirection layer where handles point to a
   forwarding table that points to the actual slot) has a constant
   per-access cost. The chosen design pays a one-time remap cost on
   promotion but no per-access tax.

The single-generation predecessor remains usable as a fallback by
configuring `GcConfig` with a very large `collect_after_allocations`
that prevents minor collection from firing. Tests that depend on the
pre-ADR-059 "collect everything, no promotion" semantics call
`Heap::collect` directly, which is preserved as the major-collect
entry point.

## Consequences

- `crates/engine-script/src/gc/{nursery,old_gen,remembered,barrier}.rs`
  go from ~10-line placeholders to substantive implementations
  (60–220 lines each).
- `crates/engine-script/src/gc/mod.rs` refactors into a generational
  façade. Public API surface preserved
  (`alloc`/`get`/`get_mut`/`collect`/`should_collect`/`live_handles`/`stats`).
  New API: `minor_collect` (returns remap table), `write_barrier`,
  `GcHandle::{is_old,is_young,index,nursery,old}`, `OLD_GEN_BIT`,
  `INDEX_MASK`.
- `GcStats` gains two counters: `minor_collections` and `promotions`.
- The pause oracle (`tests/gc_pause_oracle.rs`) is preserved; the
  histogram now reflects only minor-collect pauses once the
  dispatcher wiring (deferred to PR 0 follow-up) routes through
  `should_collect → minor_collect`. Today's oracle still calls
  `collect()` directly and remains informational.
- Aggregate opcodes (ADR-060) are the write-barrier fire sites; the
  dispatcher invocation of `Heap::write_barrier(source, target)` lands
  in the ADR-060 implementation PR alongside the opcodes.

## Risks and tradeoffs

- **Promotion remap requires dispatcher cooperation.** Today's
  dispatcher does not call `minor_collect`, so the remap table is
  exercised only by unit tests. When the dispatcher wires
  `should_collect → minor_collect` (PR 0 follow-up), every register
  in the live call stack must be rewritten via the remap. Bug surface
  for the wiring PR; the unit test
  (`promotion_after_two_minor_collects`) gives the algorithm a
  ground-truth check.
- **Card size of 512 B may be wrong for sli's actual workload.** The
  card size is a literature default, not an empirical pick. Once
  pause-oracle data accumulates, the size becomes tunable. Documented
  for the Phase-5+ tuning sweep.
- **Single u32 limits live objects to 2^31 per generation.** ~2 B
  objects per generation is well past any sli workload the spec
  contemplates. Future widening (to u64 handle) is a contract-breaking
  change shielded by ADR-012's semver discipline.
- **The pause oracle is still informational.** Hard CI gate on
  sub-millisecond pause moves from `should_fail` to `must_pass` after
  the dispatcher wiring lands and the histogram data justifies the
  threshold. Phase 0 catchup PR closes the design gap; Phase 0+
  follow-up PR activates the gate.

## Alternatives considered

- **Stay single-generation; tune `collect_after_allocations` higher.**
  Defers the pause budget without solving it; misses ADR-035's design
  intent. Rejected.
- **Refcounting instead of GC.** Cycles require a cycle collector
  anyway; refcounting's per-mutation cost is higher than card
  marking's amortised. Rejected.
- **Concurrent marking on a background thread.** Adds thread-safety
  complexity to the heap; the determinism contract (ADR-013) would
  need careful work to keep the GC's collection scheduling
  deterministic. Phase 10+ candidate, not Phase 0.
- **Per-object remembered set instead of card marking.** Per-edge
  precision; unbounded space cost under hot-loop mutation. Rejected.
- **Larger cards (4 KiB).** Fewer cards but larger scans per dirty
  card. SpiderMonkey's experience pins 512 B as the typical
  optimum; revisit empirically post-Phase-5.
- **PROMOTION_AGE = 1.** Aggressive promotion; fills old gen faster,
  defeats the generational hypothesis for the common case.

## Verification

- `crates/engine-script/src/gc/{nursery,old_gen,remembered,barrier,mod}.rs`
  each ship with unit tests:
  - `nursery::tests::{alloc_and_get, sweep_unmarked, aging_then_promotion}`
  - `old_gen::tests::{promote_and_get, major_sweep_drops_white, card_indexing}`
  - `remembered::tests::{record_and_clear, card_indexing}`
  - `barrier::tests::{classification, would_fire_only_for_old_to_young}`
  - `gc::tests::{handle_encoding_roundtrip, alloc_starts_in_nursery,
    promotion_after_two_minor_collects, write_barrier_records_old_to_young,
    major_collect_sweeps_both_gens}`
- The five existing oracle tests in `tests/gc_oracle.rs` continue to
  pass without modification.
- The pause oracle in `tests/gc_pause_oracle.rs` continues to pass
  (informational).
- The full `cargo test -p engine-script` suite (74 tests) is green.
- The dispatcher wiring + the pause-oracle gate activation are
  follow-up tasks tracked against ADR-060's implementation PR and the
  Phase-0+ catchup-closure PR respectively.
