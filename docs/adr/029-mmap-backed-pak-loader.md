# ADR-029 — mmap-backed pak loader

- **Status**: accepted
- **Phase**: 2 (Linux Systems, spec Part XXI)
- **Date**: 2026-05-19

## Context

`Pak::from_bytes(fs::read(path))` is the only load path the foundation
phase shipped. It reads the entire pak into a `Vec<u8>`, copies every blob
again into the pak's `BTreeMap<ContentHash, Vec<u8>>`, and only then
exposes `&[u8]` views to callers. For a 256 MiB shipping pak that is
~512 MiB of allocation churn and a sizeable `read(2)` storm — both visible
in the cold-load profile, and both completely unnecessary on any system
that has `mmap(2)`.

Phase 2 ("own the platform surface") is the natural place to bring zero-copy
asset loading in-tree: the OS-surface primitive is small and unavoidable
for every later phase (streaming texture loads, hot-reload, the asset
sandbox subprocesses of ADR-019, the editor's preview tooling).

## Decision

Add an owned read-only mmap wrapper in `engine_platform::mmap::MmapRo`
and an mmap-backed pak constructor `Pak::open_mmap(path)`.

### `MmapRo`

- POSIX-only (`#[cfg(target_os = "linux", target_os = "macos")]`).
  `MmapRo::open` calls `libc::mmap(NULL, len, PROT_READ,
  MAP_PRIVATE | MAP_POPULATE, fd, 0)`. `MAP_POPULATE` is Linux-only;
  macOS skips the flag. `munmap` runs in `Drop`.
- Not `Clone`. Sharing is via external `Arc<MmapRo>` so the kernel
  mapping outlives every entry that borrows from it but is `munmap`-ed
  exactly once.
- Windows: `MmapRo::open` returns `io::ErrorKind::Unsupported`. Windows
  runtime parity is deferred to Phase 11.

### `BlobSource`

A new enum in `engine_asset::store`:

```rust
pub enum BlobSource {
    Owned(Vec<u8>),
    Mapped { mmap: Arc<MmapRo>, range: Range<usize> },
}
```

Both variants expose `&[u8]` via `as_bytes()` — the choice of variant is
invisible to callers. `Pak::blobs` becomes
`BTreeMap<ContentHash, BlobSource>`; the builder path emits `Owned`, the
new `open_mmap` path emits `Mapped`.

### `Pak::open_mmap`

- Maps the file, parses the header in-place (no copies), and for every
  blob declared in the table validates `offset + len <= file_len`
  *before* indexing the mapping. A truncated or out-of-bounds pak
  surfaces as `PakError::Truncated` or `PakError::OutOfBounds` — never
  as a SIGBUS at first-touch.
- Verifies every blob's `ContentHash` (one streaming pass). Cost is
  unavoidable for the integrity contract that `from_bytes` already pays.

### Hot-reload semantics

Unchanged. `PakSet::mount(new_pak)` is still the atomic publication; a
mapping is never mutated in place, so the live overlay model from
ADR-008 carries over without change.

## Consequences

- **Load wall-clock**: a small (~10%) win on Linux for a 256 MiB pak; the
  dominant cost remains the per-blob `ContentHash::of`. The win comes
  from eliminating the `Vec<u8>` allocation and the read→memcpy chain
  into it. (See `docs/observatory/mmap-asset-baseline.md`.)
- **Resident-set behaviour — honest tradeoff**: `MAP_POPULATE` pre-faults
  every page during `open()`. RSS therefore *does not* scale with the
  working set the game actually touches; it scales with the pak file
  size. This is the deterministic-load tradeoff: without `MAP_POPULATE`,
  every first-touch read on a cold page faults in the middle of a frame,
  which is worse than upfront RSS for our hot-path use case. The win
  vs. `fs::read` is the elimination of *transient* allocation (the
  `Vec<u8>` `read(2)` target) — steady-state RSS is similar on Linux. A
  future `MmapRo::open_lazy(path)` variant is the natural place to make
  this tradeoff configurable.
- **SIGBUS safety**: the explicit `offset + len <= file_len` check makes
  the truncated-pak case a `Result` rather than a kernel signal. The
  `mmap_roundtrip` oracle pins this with a test that truncates the
  blob's tail and a synthesized OOB-declared header.
- **Windows lag**: an `Unsupported` stub keeps `engine-asset` compiling
  on Windows; callers continue to use `from_bytes(fs::read(path))` until
  Phase 11.
- **R-02 substrate**: `mmap` is the only OS surface touched in Phase 2's
  asset path — owning it leaves the runtime free of `memmap` /
  `memmap2` external crates. A CI grep rejects `libc::mmap` /
  `libc::munmap` / `libc::madvise` outside `engine-platform/src/mmap.rs`.

## References

- TLPI Ch. 49 — *Memory Mappings*. POSIX semantics for `mmap`, `munmap`,
  `MAP_PRIVATE`, `MAP_POPULATE`.
- OSTEP Ch. 18–22 — paging and the kernel page cache.
- ADR-008 — content-addressed asset pipeline (the broader load model
  this PR slots into).
- ADR-013 — Determinism Contract. The mmap path preserves the
  byte-for-byte invariant because the same `ContentHash::of(blob)` runs
  on both load paths.
