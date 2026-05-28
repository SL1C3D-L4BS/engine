# Robin Hood hash map baseline

Criterion baseline for the owned `engine_core::collections::HashMap`
(Phase 2, ADR-028). Refresh with `just hashmap-baseline` on every dev host;
commit fresh numbers here so the file accumulates history.

Each iteration is keyed by `N = 4096` `u32`s drawn from a multiplicative
congruential PRNG (seed `0xDEAD_BEEF_CAFE_F00D`). For grow-from-empty
benches the map is constructed inside the timed loop; for steady-state
benches the map is pre-populated and only the query/remove loop is timed.

## Host

- **CPU**: Intel Skylake 4c/8t @ 3.4 GHz (developer's machine)
- **Date**: 2026-05-19
- **Toolchain**: stable 1.95.0 (`rust-toolchain.toml`)
- **Profile**: `bench` (release with debug info)

## Results

Times are Criterion's median (`time:   [low mid high]`). Lower is better.

| bench                                | median time |
| ---                                  | ---:        |
| `hashmap_insert_grow_ours`           | 225.11 µs   |
| `hashmap_insert_grow_std_siphash`    | 193.06 µs   |
| `hashmap_insert_grow_std_fasthasher` | 58.07 µs    |
| `hashmap_get_hit_ours`               | 29.56 µs    |
| `hashmap_get_hit_std_siphash`        | 56.16 µs    |
| `hashmap_get_hit_std_fasthasher`     | 13.45 µs    |
| `hashmap_get_miss_ours`              | 28.65 µs    |
| `hashmap_get_miss_std_siphash`       | 52.15 µs    |
| `hashmap_remove_ours`                | 80.20 µs    |
| `hashmap_remove_std_siphash`         | 156.97 µs   |

## Notes

- **Inserts grow path** — ours edges std-SipHash (225 µs vs. 193 µs) but is
  beaten by SwissTable + FxHash (58 µs). SwissTable's metadata-byte scan
  amortizes well when grow events are amortized across many inserts; our
  table pays the cost of rehashing on each doubling. This is acceptable for
  the migration sites — none of them are insert-heavy steady-state — but
  the gap is real and we should not pretend otherwise.
- **Lookups (hit and miss)** — our table beats std-SipHash by ~2× on both
  hit and miss (29 µs vs. 56 µs, 28 µs vs. 52 µs). This is the inner-loop
  win we care about: `World::columns.get(...)` and
  `ContentStore::blobs.contains_key(...)` are point lookups on the hot
  frame path. SwissTable + FxHash still wins this microbenchmark; the
  point of ADR-028 is not raw single-thread throughput but **bounded probe
  variance** under contended / pathological workloads, which Criterion's
  uniformly-distributed key stream does not exercise. The
  probe-distance-histogram golden in `tests/collections_parity.rs` is the
  real correctness lever; this table is the bookkeeping companion.
- **Removes** — ours beats std-SipHash by ~2× on the SipHash side (80 µs
  vs. 157 µs). Backward-shift deletion does no tombstone bookkeeping; std
  with SipHash spends ~2× as long re-hashing on the probe path.
- These numbers are stable to within Criterion's reported confidence band
  on this host. Treat ±5% drift as noise; treat any consistent 2× change
  as a regression worth investigating.

## Methodology

- `cargo bench -p engine-core --bench collections` (i.e. `just hashmap-baseline`).
- Bench source: `crates/engine-core/benches/collections.rs`.
- Criterion default sampling (100 samples, 3 s warmup, 5 s analysis).
- No system tuning (no CPU pinning, no governor changes); the absolute
  numbers are not portable. The *ratio* between rows is what the baseline
  is here to track.

## Why this is *not* a CI gate

- Bench numbers are runner-noisy; CI infrastructure varies. Per the
  workspace `justfile`, benches are runnable but not part of `just ci`.
- The CI determinism story for this map is owned by the parity oracle
  (`tests/collections_parity.rs`), not by these timings.
