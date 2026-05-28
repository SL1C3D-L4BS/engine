# mmap'd-pak loader baseline

Baseline for the `Pak::open_mmap` zero-copy load path (Phase 2, ADR-029).
Refresh with `just mmap-baseline` (which runs
`cargo run --release -p engine-asset --example mmap_baseline`).

The example builds a synthetic 256 MiB pak (10 000 blobs of pseudo-random
incompressible bytes) on `$TMPDIR`, loads it through both paths, and
reports load wall-clock, RSS delta, and a small cold-vs.-warm random-read
sweep.

## Host

- **CPU**: Intel Skylake 4c/8t @ 3.4 GHz (developer's machine)
- **Date**: 2026-05-19
- **Toolchain**: stable 1.95.0 (`rust-toolchain.toml`)
- **Profile**: `release`
- **OS**: Linux 7.0.9-1-cachyos-bore
- **Backing FS**: `$TMPDIR` (tmpfs)

## Results

| measurement                              | `from_bytes(fs::read)` | `open_mmap` |
| ---                                      | ---:                   | ---:        |
| load wall-clock                          | 1.33 s                 | 1.21 s      |
| RSS delta after load                     | 263 MiB                | 263 MiB     |
| cold-cache 10-blob random-read latency   | 18.51 µs               | 14.71 µs    |
| warm-cache 10-blob random-read latency   | 2.91 µs                | 2.86 µs    |

## Notes

- **Load wall-clock** — `open_mmap` is ~10% faster than `fs::read +
  from_bytes` for this 256 MiB pak. The dominant cost in both is the same
  per-blob integrity hash (`ContentHash::of(blob)`), which mmap does not
  avoid; the win is the eliminated `Vec<u8>` allocation and the
  `read(2)` → `memcpy` chain into it.

- **RSS delta — same on both paths.** This is the trade-off ADR-029 names
  in its consequences section: we use `MAP_POPULATE`, which pre-faults
  every page during `mmap(2)`. The kernel therefore reports the same
  resident-set whether the bytes live in a `Vec<u8>` (`from_bytes`) or
  in the page cache (`open_mmap`). Without `MAP_POPULATE`, mmap RSS
  would scale with the working set the game actually touches — but every
  first-touch page-fault would land in the middle of a frame, which is a
  worse latency profile for our use case. We accept the upfront RSS
  cost in exchange for a deterministic load. (A future variant could
  expose a `MAP_NORMAL` constructor for tools that don't care about
  intra-frame latency.)

- **Cold-cache random-read latency** — `open_mmap` is ~20% faster on the
  first random touch. With `MAP_POPULATE` the kernel already faulted the
  page in during `open()`, so the "cold" measurement is in fact warm at
  the kernel level; the small remaining gap is the TLB and L1d miss on
  first touch.

- **Warm-cache random-read latency** — identical within noise. Both paths
  produce a `&[u8]` borrowed from the pak; there is no path-specific
  overhead on the read once the cache is warm.

## Methodology

- Synthetic blobs: ~26 KiB each, generated with a multiplicative
  congruential PRNG seeded with `0xDEAD_BEEF_C0DE_C0DE`. Incompressible
  by construction so tmpfs / page cache cannot "cheat" on either path.
- The from_bytes path drops the `Vec<u8>` returned by `fs::read` *before*
  the RSS measurement, so the reported number is the cost of the pak
  itself (its `BlobSource::Owned` blobs) and not the transient input
  buffer.
- The mmap path keeps the `Arc<MmapRo>` alive for the whole RSS
  measurement and the cold/warm reads.
- The pak file is `unlink(2)`-ed after each run.
- Bench numbers are noisy on shared hosts; treat single-digit percent
  drift as noise. Look for 2× changes.

## References

- TLPI Ch. 49 — *Memory Mappings*. POSIX semantics for `mmap`/`munmap`.
- OSTEP Ch. 18–22 — paging and the kernel page cache.
- ADR-029 — this baseline's design decision; see the "Tradeoffs" section
  for the MAP_POPULATE story.
