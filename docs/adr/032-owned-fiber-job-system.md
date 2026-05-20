# ADR-032 — Owned fiber job system with work-stealing thread pool

- Status: accepted
- Date: 2026-05-20
- Phase: 3 — ENGINE CORE
- Companion: ADR-031 (archetype storage), ADR-033 (parallel scheduler)

## Context

Phase 3's milestone — 1 M entities at 60 FPS on one core, with
non-conflicting ECS systems running in parallel (spec Part XXI) — needs
two new primitives the foundation layer does not have:

1. A **thread pool** the engine controls. Today `engine-platform` exposes
   `available_parallelism` and a frame-pacing helper but never spawns a
   worker thread. The ECS scheduler in Phase 1/2 runs systems
   sequentially.

2. A **job-graph dispatcher** that knows the read/write set of every
   system and runs non-conflicting jobs in parallel. The static analysis
   is the spec's "non-conflicting systems run in parallel" guarantee.

External crates that solve these problems exist (`rayon`, `crossbeam`,
`tokio`). ADR-025 forbids their inclusion: the engine ships an owned
substrate so the worst-case behaviour, scheduling fairness, and
determinism contract can all be reasoned about from one source tree.

## Decision

Ship `engine_platform::{thread_pool, jobs, fiber}` — three owned modules
that, together, give Phase 3 its compute primitive:

### Thread pool

`engine_platform::thread_pool::ThreadPool` is an N-worker pool sized to
[`std::thread::available_parallelism`](std::thread::available_parallelism)
by default. Each worker owns one OS thread spawned via
`std::thread::spawn`. Submission goes through:

- A per-worker local FIFO queue (`Mutex<VecDeque<Job>>`).
- A shared global injector queue (`Mutex<VecDeque<Job>>`).
- Idle workers steal from peers (FIFO steal, LIFO local) before parking
  on a `Condvar`.

The plan's "MPSC ArrayQueue (cap 1024) + lock-free deque" is documented
as a possible optimisation: at Phase 3's job-count scale (dozens of
systems × per-frame submissions) the lock cost is below the noise floor
of the determinism contract. A genuinely lock-free deque is a Phase 4+
concern; the API does not change either way.

### Job graph

`engine_platform::jobs::JobGraph` is a static DAG with R/W declarations
per node:

- `JobGraph::add_job(reads: &[u64], writes: &[u64], body)` registers a
  job. Keys are opaque `u64` — the ECS scheduler feeds
  `TypeStableId::as_u64()` (ADR-031) into them; the pool sees only
  numbers.
- `JobGraph::run()` runs in registration order on the calling thread —
  the deterministic reference path the oracle compares against.
- `JobGraph::run_on(&pool)` builds a dependency graph from R/W
  intersections (`A.writes ∩ B.reads ≠ ∅` ⇒ edge `A→B` for `A` registered
  before `B`), kicks off every zero-in-degree job into the pool's
  injector, and waits on a `Condvar` until every job has run.

Determinism is established at the graph level (R/W-disjoint jobs commute,
so final state is order-independent), not at the work-stealing level —
which lets us avoid the wider cost of forcing a fixed steal order.

The dispatcher snapshots the initial zero-in-degree set *before* the
in-degree counters become atomic. Without this snapshot, the main
thread's initial scan could race with a worker's recursive
`spawn_job(j)` and end up enqueuing `j` twice — the bug the R-02 oracle
flushed out during PR 2's first parallel run.

### Fibers

`engine_platform::fiber::{Fiber, switch}` is a stack-bound, cooperatively
scheduled coroutine primitive:

- 64 KiB usable stack (default) backed by [`MmapAnon`] with a one-page
  `PROT_NONE` guard at the low address — overflow segfaults the offending
  thread, never trashes adjacent memory.
- Context switching via naked asm on x86-64 (callee-saved registers +
  `rsp` + `rip`) and aarch64 (`x19`–`x29`, `lr`, `sp`). A `ucontext.h`
  fallback compiles on unsupported architectures but `panic!`s if
  invoked.

Phase 3 PR 2 ships fibers as a primitive but does *not* wire them into
the scheduler. The static-DAG, R/W-disjoint job model is
run-to-completion per job — fibers matter only when a job genuinely
needs to yield (long-running async work, frame-spanning streams). That's
a Phase 4+ concern. Exporting the primitive now means the rest of the
engine has a stable shape to compose against without re-doing register
asm.

## Consequences

### Positive

- The engine now has an owned compute substrate. Every threading
  primitive in the runtime traces back to one of three files under
  `engine_platform/`.
- The R-02 oracle (`tests/jobs_oracle.rs`) holds the contract: parallel
  execution at `{1, 2, 4, N}` workers produces the same final state
  digest as the single-threaded reference path, across pseudo-random
  DAGs of 32–64 jobs.
- Determinism is preserved by construction at the level the spec names —
  R/W-disjoint commutativity — not by enforcing a fixed steal order
  (which would lose most of the parallel speedup).

### Negative

- The Mutex-protected per-worker deque is contended under heavy
  fanout. The plan calls out a lock-free per-worker deque as a possible
  optimisation; ADR-032 documents the upgrade path without committing
  to a timeline. The Phase 3 milestone (1 M entities, 60 FPS) is the
  forcing function; if the bench misses, the deque is the first place
  to look.
- Fiber switching is unimplemented on Windows (the ABI is different and
  Phase 3 does not target Windows). The `ucontext` fallback `panic!`s if
  invoked — programs that compile on Windows will fail at runtime the
  moment a fiber is constructed.
- The `Pool::submit` and `ThreadPool::pool_arc` crate-private API surface
  exists so `JobGraph::run_on` can submit jobs onto the pool from inside
  a job's closure without going through a `&ThreadPool` borrow that
  isn't `Send`. The ergonomic cost of routing the `Arc<Pool>` everywhere
  is small; the alternative was an `unsafe Send` wrapper around a raw
  pointer, which the security review (ADR-014) would object to.

### Neutral

- `std::thread::spawn`, `std::sync::Mutex`, and `std::sync::Condvar` are
  used by `thread_pool.rs` and grep-prohibited outside the allowlist
  (`thread_pool.rs` + `sampler.rs`). The rest of the engine routes
  through the pool.

## Files changed

- `crates/engine-platform/src/thread_pool.rs` (new) — `ThreadPool`,
  `Pool`, worker loop.
- `crates/engine-platform/src/jobs.rs` (new) — `JobGraph`, `JobId`,
  conflict detection, recursive dispatch.
- `crates/engine-platform/src/fiber/mod.rs` (new) — `Fiber`, public
  switch entry point.
- `crates/engine-platform/src/fiber/x86_64.rs` (new) — `Context`,
  naked-asm switch (System V).
- `crates/engine-platform/src/fiber/aarch64.rs` (new) — `Context`,
  naked-asm switch (AAPCS64).
- `crates/engine-platform/src/fiber/fallback.rs` (new) — `ucontext.h`
  stub for unsupported targets.
- `crates/engine-platform/src/mmap.rs` — `MmapAnon` extension with
  optional `PROT_NONE` guard page.
- `crates/engine-platform/src/lib.rs` — module wiring + re-exports.
- `crates/engine-platform/tests/jobs_oracle.rs` (new) — R-02 oracle
  (parallel vs single-threaded digest equality).
- `crates/engine-platform/Cargo.toml` — `blake3` dev-dep (oracle uses
  it).
- `.github/workflows/ci.yml` — guards against `std::thread::spawn`,
  `std::sync::{Mutex,RwLock,mpsc}` outside `thread_pool.rs` /
  `sampler.rs`; rejects `rayon`/`crossbeam`/`tokio`/`async-std`/
  `parking_lot` substrings anywhere in `crates/`.
- `justfile` — `jobs-oracle` and `jobs-bench` recipes.
- `docs/observatory/jobs-baseline.md` (new) — informational
  benchmark placeholder; not a CI gate.
