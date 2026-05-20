# [ENGINE] Platform

A Rust-first, zero-runtime-dependency game engine and platform.

This repository is the monorepo through **Phase 1** (SILICON → C). The crate
tree, levels, and architecture follow the authoritative specification:

> `~/Resources/documentation/ENGINE_SPECIFICATION_v2.0.md`

## Layout

- `crates/` — the 19 `engine-*` library crates (levels 0–4, spec Part IV.1)
- `testbed/engine-raster/` — software rasterizer, the rendering oracle (Part IX)
- `tools/`, `bin/` — CLI and TUI tool crates (Parts VII, VIII)
- `tests/{integration,rendering,semver,bench,determinism}/` — test surfaces
- `docs/adr/` — Architecture Decision Records (Part XXII)
- `docs/architecture/` — subsystem architecture docs

## Build

```sh
just build      # compile the workspace
just ci         # full pre-push gate (build, test, lint, fmt, deny)
```

The toolchain is pinned in `rust-toolchain.toml`; `sccache` and the `mold`
linker are configured in `.cargo/config.toml`.

## Status

**Foundation layer built, machine model owned.** The 8 dependency-Level-0
and Level-1 crates are real, tested, and oracle-verified:

- Level 0 — `engine-math`, `engine-platform`, `engine-reflect`,
  `engine-ecs-macro`
- Level 1 — `engine-core`, `engine-asset`, `engine-telemetry`, `engine-i18n`

Phase 1 (SILICON → C, spec Part XXI) added the three portfolio deliverables
under the foundation:

- **SIMD math.** `engine-math` arithmetic, dot, and matrix multiplies now run
  through a private four-lane SIMD wrapper (SSE2 on x86-64, NEON on aarch64,
  scalar fallback elsewhere). The cross-architecture determinism digest is
  unchanged; a 100,000-input parity oracle locks the SIMD path to the frozen
  scalar reference byte-for-byte (ADR-027).
- **Arena allocator.** A fourth general-purpose free-list arena lives
  alongside the linear, ring, and pool arenas; every arena now exposes a
  uniform `Arena` accounting trait that `engine-memdbg` (Phase 2) will
  consume (ADR-026). Criterion baseline in
  `docs/observatory/arena-baseline.md`.
- **Cache observatory.** A standalone CLI at `tools/cache-observatory/`
  sweeps four workloads (Vec3 traversal, hot/cold contrast, Mat4 chain,
  LinearArena random reads) over working sets from 4 KiB to 64 MiB and
  reports wall-clock plus optional `perf_event_open` cache-miss counts.
  Layout-invariant `const _: () = assert!` tripwires in `Vec3`, `Vec4`,
  `Mat3`, `Mat4`, `Quat`, and `Entity` fail the build on layout drift.
  Baseline: `docs/observatory/cache-baseline.md`.

Each ships a verification oracle (spec R-02); the cross-architecture
Determinism Contract is enforced by `.github/workflows/ci.yml` (`just
determinism`). The upper layers (render, physics, audio, net, script, ai,
editor, …) remain stubs and are built across the later phases — see spec
Part XXI.
