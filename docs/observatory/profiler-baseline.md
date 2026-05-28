# Sampling profiler baseline

Baseline for `engine_telemetry::SamplingProfiler` (Phase 2, ADR-030).
Refresh with `just profiler-baseline` after every meaningful change.

The CLI tool (`tools/sampling-profiler/`) drives the profiler against
two built-in workloads:

- `spinner` — tight ALU loop (pure on-CPU work).
- `arena_alloc` — `engine_core::alloc::LinearArena` construct-then-fill,
  similar to the Phase 1 arena bench.

## Host

- **CPU**: Intel Skylake 4c/8t @ 3.4 GHz (developer's machine)
- **Date**: 2026-05-19
- **Kernel**: Linux 7.0.9-1-cachyos-bore
- **`perf_event_paranoid`**: 2 (default; user-only sampling)
- **Toolchain**: stable 1.95.0 (`rust-toolchain.toml`)
- **Profile**: `release`
- **RUSTFLAGS**: `-C force-frame-pointers=yes` (mandatory; ADR-030)

## Sample capture rate

The CLI tool requests a target rate and reports actual capture
coverage. `--duration-s 1`, `--workload spinner` (pure on-CPU work,
the most favourable case for the kernel scheduler):

| target rate | captured | coverage | dropped | stacks |
| ---:        | ---:     | ---:     | ---:    | ---:   |
|   99 Hz     |   98     | 99.0 %   |   0     | 4      |
|  199 Hz     |  198     | 99.5 %   |   0     | 4      |
|  499 Hz     |  292     | 58.5 %   |   0     | 5      |
|  997 Hz     |  292     | 29.3 %   |   0     | 3      |

The kernel throttle (`kernel.perf_cpu_time_max_percent`, default 25 %)
caps total CPU time spent in perf to a fraction of wall-clock; with the
ring buffer never overflowing (`dropped = 0` across the board) the
shortfall above ~200 Hz is the throttle, not the sampler. Raising
`perf_cpu_time_max_percent` on the host shifts the ceiling
proportionally; the sampler itself happily handles 1 kHz on its end.

On the `arena_alloc` workload, capture rate falls off slightly sooner
because the workload spends part of every iteration off-CPU in the
arena's allocation bookkeeping:

| target rate | captured (2 s) | coverage |
| ---:        | ---:           | ---:     |
|   99 Hz     |  197           | 99.5 %   |
|  199 Hz     |  274           | 68.8 %   |
|  499 Hz     |  276           | 27.7 %   |
|  997 Hz     |  276           | 13.8 %   |

## Self-overhead

`engine_telemetry::SamplingProfiler::try_attach + finish` adds the
following overhead in the `arena_alloc` workload — the same workload
the Phase 1 arena bench uses, so the numbers are comparable to
`docs/observatory/arena-baseline.md`:

- **`try_attach(99)`**: one `perf_event_open` syscall + one `mmap`
  call. Sub-millisecond on every host the engine targets.
- **`finish()`**: one `ioctl(PERF_EVENT_IOC_DISABLE)` + a single
  `read`-cursor scan of the mmap'd ring. Cost scales with the number
  of unhandled samples; for the ranges in this baseline it is in the
  low microseconds.
- **In-loop overhead**: the kernel writes one record per sample; the
  user-space cost of consuming each record is dominated by the
  `HashMap` insert into the folding table. With the
  [Robin Hood map][adr-028] this is a constant ~80 ns per sample on
  this host.

Holding both the sampler **and** the workload-bounded measurement
loop at fixed duration, the per-iteration time difference between
sampled and unsampled `arena_alloc` runs is within Criterion's noise
band — the profiler does not measurably slow the workload at 99 Hz
or 199 Hz, the rates the runtime engine will use in practice.

## Methodology

- `cargo build --release -p sampling-profiler` with
  `RUSTFLAGS="-C force-frame-pointers=yes"`.
- Each row above is a single `./target/release/sampling-profiler …`
  invocation; numbers are stable within ±5 % on this host between
  back-to-back runs.
- Sample drops at high rates are an artefact of the kernel's
  CPU-time throttle, *not* of the ring buffer; check
  `kernel.perf_cpu_time_max_percent` if raising the per-fd rate is
  desired.

## Why this is *not* a CI gate

- The exact number of captured samples is host-dependent (kernel
  build, throttle settings, scheduler).
- The oracle (`crates/engine-telemetry/tests/profiler_oracle.rs`)
  pins the *correctness* of the profiler with a synthetic workload;
  CI runs the oracle with `--release` and the frame-pointer flag.
  This file is the *bookkeeping* companion — refresh when the
  producer or consumer changes shape.

[adr-028]: ../adr/028-owned-robin-hood-hash-map.md
