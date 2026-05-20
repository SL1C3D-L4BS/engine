# ADR-026 · General free-list arena and unified arena accounting

- Status: Accepted
- Date: 2026-05-19
- Phase: 1

## Context

`engine-core::alloc` shipped three arenas in Phase 0 — `LinearArena`,
`RingArena`, and `PoolArena` — covering the per-frame scratch, the rolling
history, and the generational pool use cases respectively. Spec XVI calls for
a fourth: a general-purpose free-list allocator for patterns that are too
irregular for a bump arena and not type-uniform enough for a pool. Phase 0
deferred it explicitly with a `// the global allocator serves that role`
comment in `alloc.rs`.

Phase 1 needs the fourth arena for two concrete reasons:

1. **Cache observatory workloads.** The `linear_arena_random_reads` workload
   under `tools/cache-observatory/` allocates many odd-sized records and
   later releases a subset; the bump arena can model the fill but not the
   churn. Other workloads we will add in Phase 2 (texture staging, asset
   hot-load scratch, the script-VM heap) are even less uniform.
2. **A clean target for the memory debugger.** `engine-memdbg` (Phase 2,
   spec XVI) needs a uniform accounting interface across every arena so it
   can present per-subsystem watermarks in the editor. With three different
   arenas exposing three different field sets, the memdbg integration would
   begin life as a special-case-per-arena nightmare.

These two needs are independent but well-aligned: the same Phase 1 PR that
lands the fourth arena should also unify the accounting surface.

## Decision

### A `GeneralArena` with segregated size classes plus a coalescing large list

`GeneralArena::with_capacity_named(bytes, name)` carves a fixed byte buffer
and routes allocations by request size:

- `len <= 4096` → one of eight segregated size classes
  (`16, 32, 64, 128, 256, 512, 1024, 2048, 4096`). Each class has its own
  free list whose nodes are embedded in freed blocks (the freed block holds a
  `BlockHeader` whose `next` field threads the list).
- `len > 4096` → a coalescing large list. First-fit over the available
  blocks, with an oversized hit split into the requested block + a remainder
  pushed back onto the list. On free, a sort-and-merge sweep coalesces
  physically adjacent blocks back into larger ones.

Allocations return `None` on exhaustion. The arena never grows.

The size-class set is the conventional one — small enough to keep the
internal-fragmentation bound visible (worst case: a 4097-byte request would
need to fall through to the large list, an explicit cost), large enough that
the small-block path costs O(1) head-of-list pops.

The coalesce sweep is intentionally O(n²) in the number of outstanding large
blocks. For the workloads expected on this arena (a few thousand outstanding
large blocks at most, typically under 100) that is well under a microsecond;
the simplicity buys clarity at no measured cost. If a future workload changes
this, the sweep is a localised replacement.

### A uniform `Arena` trait

```rust
pub struct ArenaStats {
    pub name: &'static str,
    pub used: usize,
    pub capacity: usize,
    pub peak: usize,
    pub allocations: u64,
    pub frees: u64,
    pub resets: u64,
}

pub trait Arena {
    fn stats(&self) -> ArenaStats;
    fn name(&self) -> &'static str;
}
```

All four arenas implement it. The `with_capacity_named(cap, name)`
constructors are `#[track_caller]` so `engine-memdbg` can attribute the
construction site of every arena to a source location in Phase 2 — at no
runtime cost in Phase 1.

The trait is intentionally minimal. The arenas have widely different
allocation APIs (`alloc(len, align)` vs `push(value)` vs `insert(value)`),
and a unified `alloc` method would either over-fit one shape or under-fit
all of them. Accounting is the *one* thing every arena does the same way.

### What we do NOT do in Phase 1

- No global registry of live arenas. The `Arena` trait is the interface;
  `engine-memdbg` will be the registry in Phase 2.
- No global allocator hook. `ENGINE_ALLOC_TRACK=1` is spec XVI's Phase 2
  feature and stays deferred.
- No criterion benches in `just ci`. Bench numbers are too runner-noisy to
  fail a CI build on; they are tracked manually in
  `docs/observatory/arena-baseline.md` and re-run on demand via `just bench`.

## Consequences

- A fourth arena to maintain. Test coverage spans size-class round-trip,
  every-class allocation, large-block first-fit + coalesce, exhaustion, and
  reset.
- `engine-memdbg` (Phase 2) lands additively. It can iterate any registered
  `dyn Arena` without per-arena special-casing.
- Existing callers of `LinearArena::with_capacity` / `RingArena::with_capacity`
  / `PoolArena::new` keep working unchanged; the unified constructors layer
  on the named variants with a default static label.
- The `criterion` dev-dependency joins the workspace. It is only pulled in
  through `[dev-dependencies]`, so the release dependency graph is
  unaffected; `cargo deny` runs against the shipping graph, not benches.
- Internal-fragmentation worst case is one size class round-up — for a
  17-byte allocation we charge 32 bytes plus a 16-byte header. This is the
  documented cost of having a fast small-block path.
