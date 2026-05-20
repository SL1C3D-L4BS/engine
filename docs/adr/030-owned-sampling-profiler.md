# ADR-030 — Owned sampling profiler

- **Status**: accepted
- **Phase**: 2 (Linux Systems, spec Part XXI)
- **Date**: 2026-05-19

## Context

Every later spec deliverable that *consumes* CPU profiling data —
`engine-profiler-tui` (spec VIII.2), the memory debugger (`engine-memdbg`,
spec VIII.7), the editor's hot-path inspector, the CI flamegraph
ingestion the studio runs on every merge — needs an in-process sampling
profiler that the engine can attach to its own threads. The alternatives
were:

- **`perf record` + `addr2line`** out-of-process. Familiar, but means
  the engine carries no profiling story of its own; an editor screenshot
  of "this frame's hot path" requires the user to know about `perf`,
  install it, configure paranoia, and post-process the result.
- **An existing crate** (`pprof-rs`, `profiler`, `tracy_client`).
  Spec R-02 keeps the foundation layer owned-in-tree; the substrate that
  every downstream debug tool sits on is exactly the kind of dependency
  we want to own.

## Decision

Ship an owned sampling profiler in two crates:

- **`engine_platform::sampler`** — the producer. Linux-only via
  `perf_event_open` (`PERF_TYPE_SOFTWARE` + `PERF_COUNT_SW_CPU_CLOCK`,
  `PERF_SAMPLE_IP | PERF_SAMPLE_TID | PERF_SAMPLE_CALLCHAIN`). One fd
  per attached thread (`pid = 0, cpu = -1`); samples land in an mmap'd
  ring buffer the consumer drains non-blockingly. macOS/Windows compile
  to a stub whose `try_open` returns `Ok(None)` — mirroring the
  graceful-degradation pattern the Phase 1 `LinuxPerfCounters` already
  established.
- **`engine_telemetry::profiler`** — the consumer. `SamplingProfiler`
  attaches a producer to the calling thread; `finish()` drains the
  ring, folds every IP chain into a `Vec<u64> -> count` table, and
  emits one `Signal::Sample { stack_id, count }` per unique chain so
  the existing collector / IPC / metrics-endpoint plumbing picks the
  data up without further changes.

`tools/sampling-profiler/` is the CLI: a hand-rolled-args binary that
runs a built-in workload (`spinner`, `arena_alloc`), produces
Brendan-Gregg-compatible folded-stack output on stdout, and reports
self-overhead on stderr.

### Why not SIGPROF / setitimer

The `setitimer(ITIMER_PROF) + SIGPROF` approach is the textbook
single-threaded sampler. It is rejected here because:

1. **`async-signal-safe` constrains the handler.** TLPI Ch. 21 §21.1.2
   enumerates the functions that may be called from a signal handler;
   `malloc`, `pthread_mutex_lock`, and `std::collections::HashMap::insert`
   are all on the *forbidden* side. The handler may therefore not
   resolve symbols, push to a `Vec`, or update any state that locks.
2. **Process-wide delivery is biased.** `ITIMER_PROF` fires once and
   the kernel picks *one* thread to deliver `SIGPROF` to. That biases
   samples toward whichever thread is on-CPU when the timer expires;
   for the fiber-runner workloads of Phase 3 it means the spinner
   thread is sampled, the cooperative scheduler thread is not.

`perf_event_open` with per-thread fds avoids both problems: the kernel
buffers samples into a ring buffer asynchronously, and per-thread fds
give us per-thread attribution.

### Frame-pointer codegen

`PERF_SAMPLE_CALLCHAIN` uses the kernel's frame-pointer walker by
default. Without `-C force-frame-pointers=yes` every captured chain
truncates at the leaf, which silently degrades the oracle. The CI
gate's profiler-oracle step explicitly verifies `RUSTFLAGS` contains
the flag before running the test; the `just profiler-oracle` recipe
sets the same flag locally.

Owned DWARF (`.eh_frame`) unwinding is a future enhancement — explicitly
*not* in scope for Phase 2 because it duplicates the cache-observatory's
budget and is irrelevant on the engine's optimized hot paths (which all
ship frame pointers anyway).

### Linux-only

The producer compiles to a stub on macOS / Windows; the consumer
delegates and reports the absence the same way `LinuxPerfCounters`
already does in `tools/cache-observatory/src/perf.rs`. The engine
never refuses to start because the profiler could not attach. Apple's
Instruments and Windows' ETW are the natural future producers; both
land later because they pull in platform-specific crate dependencies
that Phase 2 deliberately defers.

## Consequences

- The hot-path inspector and the CI flamegraph pipeline can build on
  a foundation that does not depend on an external `perf` binary or
  symbolization toolchain.
- One additional substrate is owned (the `perf_event_open` + ring-buffer
  + folded-stack triple); CI guards the frame-pointer requirement so
  silent degradation is impossible.
- A new `Signal::Sample` variant routes folded counts through the
  existing telemetry plumbing — IPC, metrics, and the log writer all
  see sampling data without their own ingestion paths.
- The producer is Linux-only for Phase 2. macOS/Windows parity is
  deferred behind the same `Ok(None)` graceful-degradation pattern
  already in use.

## References

- TLPI Ch. 21 §21.1.2 — *Async-signal-safe functions*. The forbidden-
  list for `SIGPROF` handlers — the reason `perf_event_open` is the
  right primitive.
- TLPI Ch. 22 — *Signals: advanced features*. Background on
  `ITIMER_PROF` and its per-process thread-selection behaviour.
- OSTEP Ch. 26–28 — concurrency. The SPSC ring-buffer parsing in
  `engine_platform::sampler::drain` is the classic lock-free
  reader / writer-cursor pair.
- ADR-013 — Determinism Contract. The profiler is wall-clock-driven
  and is therefore expected to vary; we do not pin its output in any
  cross-arch oracle.
- ADR-028 — Owned Robin Hood hash map. The folded-stack table uses it.
- `tools/cache-observatory/src/perf.rs` — the existing `perf_event_open`
  bindings the sampler reuses idioms from.
