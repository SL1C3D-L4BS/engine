# arena allocator baseline

Criterion baseline for the engine-core arenas (Phase 1, ADR-026). Re-run via
`just bench` on every dev host; commit fresh numbers here so the file
accumulates history.

Each iteration constructs a fresh arena (so the setup of the backing buffer
is part of the measurement) and runs the documented workload. Numbers are
therefore not directly comparable between benches; they are meaningful as a
trend over time, not as absolute throughput.

## Host

- **CPU**: Intel Skylake 4c/8t @ 3.4 GHz (developer's machine)
- **Date**: 2026-05-19
- **Toolchain**: stable 1.95.0 (`rust-toolchain.toml`)
- **Profile**: `bench` (release with debug info, FMA not disabled — these are
  not determinism oracles)

## Results

| bench | median time | per-op (approx) | workload |
| --- | ---: | ---: | --- |
| `linear_bump_64b` | 22.74 µs | ~22 ns | 1024 × `alloc(64, 8)` then drop |
| `linear_bump_mixed_alignment` | 21.61 µs | ~42 ns | 512 × `alloc` cycling six (size, align) pairs |
| `ring_push_steady_state` | 3.69 ns | 3.69 ns | One `push` on a full 256-element ring |
| `pool_insert_remove_churn` | 1.92 µs | ~5 ns | 256 insert / 128 remove / 128 insert |
| `general_size_class_walk` | 22.63 µs | ~1.3 µs | One alloc-free pair per size class (9 pairs) |
| `general_fragmentation_pattern` | 23.64 µs | ~92 ns | 128 × 256-byte alloc, free every other, refill 64 |
| `general_reset_after_churn` | 23.60 µs | ~92 ns | 256 × 64-byte alloc + 1 reset |

## Notes

- `linear_bump_64b` and `linear_bump_mixed_alignment` are dominated by the
  1 MiB `Vec<u8>` zero-init in each iteration (the arena's buffer). The bump
  loop itself is well under a nanosecond per alloc; the per-op figure above
  charges every iteration with arena construction.
- `ring_push_steady_state` is the cleanest microbenchmark — no arena setup
  in the hot loop. 3.7 ns/push is a `VecDeque::pop_front` + `push_back`.
- `pool_insert_remove_churn` re-uses a single pool across the iteration body,
  so its setup cost is amortised; the ~5 ns/op number is realistic.
- `general_size_class_walk` exercises every one of the nine size classes
  exactly once. Most of the time is arena construction (1 MiB) and the
  coalesce sweep on free; the per-allocation cost is sub-microsecond and
  expected to drop dramatically once the coalesce sweep moves out of the
  free hot path (a future, optional optimisation tracked outside Phase 1).
