# cache-observatory

A small Linux-only CLI that measures cache behaviour for the engine's hot data
types: `Vec3`, `Vec4`, `Mat4`, and `LinearArena`. Built for Phase 1 of the
[ENGINE] platform (spec XXI, "SILICON → C"): the goal is verified
understanding of the machine model, not just intuition.

Run from the workspace root:

```sh
# Wall-clock only.
just cache-baseline > docs/observatory/cache-baseline.md

# With kernel perf counters (L1d / L2 / LLC misses, cycles, instructions).
# Requires perf_event_paranoid <= 2 or CAP_PERFMON. Falls back to wall-clock
# only if the kernel refuses.
just cache-baseline-with-counters > docs/observatory/cache-baseline.md
```

Pass `--only <workload>` to run a single workload:

```sh
cargo run --release -p cache-observatory -- --only vec3_array_traversal
```

## Workloads

| Workload | What it measures |
| --- | --- |
| `vec3_array_traversal` | Streamed sum of `Vec<Vec3>`. The un-padded 12-byte element is the canonical sequential-read benchmark. |
| `hot_cold_parallel` / `hot_cold_interleaved` | The same data laid out as parallel arrays vs interleaved with a 64-byte cold payload — direct evidence for ADR-014. |
| `mat4_chain` | Sequential `Mat4` multiplies (one cache line per element); models the renderer's per-instance transform array. |
| `linear_arena_random_reads` | Pointer-chase a `LinearArena` via a deterministic-seed shuffled index list — shows the cost at each cache level. |

Working-set sizes sweep 4 KiB → 64 MiB doubling. Inputs are generated from the
deterministic [`engine_core::rng::Rng`] with a fixed seed, so the data layout
is reproducible host-to-host and the only run-to-run variable is hardware.

## Output

Markdown on stdout. Every report starts with a hardware fingerprint pulled
from `/proc/cpuinfo` and `/sys/devices/system/cpu/cpu0/cache/`. Redirect the
output to `docs/observatory/cache-baseline.md` to refresh the committed
baseline — that file accumulates history, so old entries should stay.

## Design notes

- **Owned everything.** No `clap`, no `perf-event`, no `criterion`. Hand-rolled
  argument parsing, `std::time::Instant` for wall-clock, direct `libc::syscall`
  for perf counters. R-02 of the spec.
- **Always-on safety net.** The wall-clock path runs on every host; the
  perf-counter path is purely additive. If `perf_event_open` returns `EACCES`
  the tool prints a note and continues — there is no fatal-on-counter-missing
  failure mode.
- **Determinism, not microbenchmark realism.** Inputs are reproducible across
  runs. Numbers will vary between hosts and between thermal states; that is
  the point. The cache-line transitions are what we read off the table, not
  absolute timings.
