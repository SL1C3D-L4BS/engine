# [ENGINE] Platform

A Rust-first, zero-runtime-dependency game engine and platform.

This repository is the monorepo through **Phase 4** (SCRIPTING), audited
and gated for Phase 5 (RENDERING FOUNDATION). The crate tree, levels,
and architecture follow the authoritative specification:

> `~/Resources/documentation/ENGINE_SPECIFICATION_v2.0.md`

## Layout

- `crates/` — the 19 `engine-*` library crates (levels 0–4, spec Part IV.1)
- `testbed/engine-raster/` — software rasterizer, the rendering oracle (Part IX)
- `tools/`, `bin/` — CLI and TUI tool crates (Parts VII, VIII)
- `tests/{integration,rendering,semver,bench,determinism}/` — test surfaces
- `docs/adr/` — Architecture Decision Records (Part XXII)
- `docs/architecture/` — subsystem architecture docs
- `docs/audit/` — enterprise-grade audits (see [Audits](#audits))
- `docs/observatory/` — performance baselines per shipped subsystem
- `docs/runbooks/` — operational runbooks for CI / infra

## Audits

- [Phases 0–4 enterprise audit (2026-05-24)](docs/audit/phases-0-4-enterprise-audit.md)
  — full audit against the spec, all 38 prior ADRs, the actual code,
  and the book library. Surfaces 14 remediation ADRs (ADR-039 through
  ADR-053) and three doc-staleness fixes that close before Phase 5
  implementation begins.

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
determinism`).

Phase 2 (LINUX SYSTEMS, spec Part XXI) added three portfolio deliverables
that own the platform surface every later phase will sit on:

- **Robin Hood hash map.** An owned open-addressed table with
  backward-shift deletion in `engine_core::collections`, with two owned
  hashers — `FastHasher` (FxHash-style multiplicative) for hot lookups
  and `DeterministicHasher` (BLAKE3-keyed) for cross-arch reproducible
  probe order. Replaces `std::collections::HashMap` across `engine-core`
  and `engine-asset` (the ECS resource lookup uses the deterministic
  variant; the rest use FastHasher). Parity oracle vs `std` and a naive
  reference; baseline at `docs/observatory/hashmap-baseline.md`
  (ADR-028).
- **mmap'd asset loader.** `engine_platform::mmap::MmapRo` wraps
  `libc::mmap` (POSIX-only, Linux/macOS; Windows returns `Unsupported`).
  `engine_asset::Pak::open_mmap(path)` opens a pak archive zero-copy via
  a `BlobSource::Mapped` enum that lets every entry borrow a sub-range
  of one shared mapping. Truncated and out-of-bounds paks surface as
  `PakError::Truncated` / `PakError::OutOfBounds` rather than SIGBUS.
  Baseline: `docs/observatory/mmap-asset-baseline.md` (ADR-029).
- **Sampling profiler.** Linux `perf_event_open` producer
  (`engine_platform::sampler`) feeds a folded-stack consumer
  (`engine_telemetry::profiler::SamplingProfiler`) that emits one
  `Signal::Sample { stack_id, count }` per unique call chain. The
  `tools/sampling-profiler/` CLI prints Brendan-Gregg-compatible folded
  stack output. macOS/Windows compile to graceful-degradation stubs.
  Baseline: `docs/observatory/profiler-baseline.md` (ADR-030).

Phase 3 (ENGINE CORE, spec Part XXI) closed the substrate with three
deliverables that wire the data layer and the compute layer into a
deterministic parallel scheduler:

- **Archetype ECS.** Table components now live in archetype-grouped
  columns (one `AnyVec` per component type per archetype) — the
  Structure-of-Arrays layout the cache-observatory workloads were
  designed to predict for. The archetype index keys signatures by a
  cross-architecture-stable `TypeStableId` (BLAKE3 of
  `crate_name::ident`, emitted as a literal `u64` by
  `#[derive(Component)]`); `std::any::TypeId` is no longer used for
  component identity anywhere in the ECS, enforced by a CI grep guard.
  Oracle: `crates/engine-core/tests/archetype.rs`. Baseline:
  `docs/observatory/archetype-baseline.md` (ADR-031).
- **Owned fiber job system.** `engine_platform::ThreadPool` is an owned
  N-worker pool (one `std::thread::spawn` per worker, allowlisted to
  one file) with per-worker FIFO deques and a shared injector; idle
  workers steal from peers before parking on a condvar. The companion
  `engine_platform::fiber` module ships guarded-stack user-space
  fibers (naked asm switch for x86-64 + aarch64, `ucontext` fallback)
  built on a new `MmapAnon` anonymous-mapping primitive.
  `engine_platform::JobGraph` provides static R/W-DAG dispatch; the
  R-02 oracle hashes parallel results against the single-threaded
  reference at worker counts {1, 2, 4, N}. Baseline:
  `docs/observatory/jobs-baseline.md` (ADR-032).
- **Deterministic parallel scheduler.**
  `Schedule::add_system_with_access(phase, name, reads, writes, fn)`
  registers parallelisable systems with explicit `TypeStableId` R/W
  sets; `Schedule::run_on(world, pool)` builds one `JobGraph` per
  phase and dispatches non-conflicting systems through the owned
  thread pool. The replay-parity oracle
  (`crates/engine-core/tests/replay_parity.rs`) runs the same workload
  sequentially and at worker counts {1, 2, 4, N} and asserts identical
  per-frame BLAKE3 digests — wired into the CI determinism job on
  both x86-64 and aarch64. Milestone bench
  (`cargo bench -p engine-core --bench million_entities`) tracks the
  1 M-entity / 60 FPS single-core target. Baseline:
  `docs/observatory/million-entities-baseline.md` (ADR-033).

**Engine Core v0.1** is tagged at the close of Phase 3.

Phase 4 (SCRIPTING, spec Part XXI) closed on 2026-05-20 across four PRs,
each gated by the cross-arch determinism oracles and the existing CI
guards:

- **sli compiler front-end.** `crates/engine-script/` ships the hand-
  written lexer + Pratt parser + bottom-up type checker + SSA IR with
  const-fold / CSE / DCE optimisation passes and a deterministic IR
  text serialiser. Oracle: `tests/compile_parity.rs` (BLAKE3 digest
  over the optimised-IR serialisation of a curated corpus, committed
  golden at `tests/goldens/sli-compile.golden`). CI grep guard rejects
  every vendored interpreter / parser-generator family
  (rlua/mlua/wasmtime/wasmer/cranelift/inkwell/lalrpop/pest/nom/
  combine/chumsky) under `crates/engine-script/`. ADR-034.
- **Register VM + tri-color GC + bytecode verifier.** Owned register-
  based dispatch with the TRAP opcode (0xFF) reserved for breakpoints
  defended by a four-layer impossibility argument (type system + grep
  guard + 500-program fuzz oracle + verifier). Single-generation GC
  ships in PR 2; the nursery / old-gen / remembered-set / write-
  barrier modules are typed stubs for the generational follow-up.
  Oracles: `tests/vm_oracle.rs`, `tests/verifier.rs`, `tests/gc_oracle.rs`,
  `tests/codegen_no_trap.rs`. ADR-035.
- **Hot-reload + debugger protocol + REPL.** Owned binary wire
  protocol (no Microsoft DAP, no serde) in `debug_proto.rs` with
  every request/response/event variant locked by a round-trip
  oracle. `bin/engine-debug/` is the debugger server; `bin/engine-repl/`
  is the cooked-mode REPL. Hot-reload uses a deterministic polling
  watcher; breakpoint persistence uses an owned TOML writer/reader
  (acknowledged deviation from spec, see ADR-051). ADR-036.
- **Slang shader toolchain.** `tools/engine-shader/` wraps the
  official `slangc` binary as a sandboxed subprocess per ADR-019,
  pinned at `SLANGC_PIN = "v2026.9"`. Owned artefact bundle format,
  content-addressed via the asset pipeline (ADR-008). The per-target
  reproducibility golden (`triangle-reproducibility.golden`) runs
  cross-arch in CI with graceful skips for unavailable backends.
  ADR-037 + ADR-038.

**Phases 0–4 enterprise audit** closed 2026-05-24
([report](docs/audit/phases-0-4-enterprise-audit.md)). The audit
landed 14 remediation ADRs (ADR-039 through ADR-053) covering the
Phase 5 design contracts (render graph, CSM, IBL, TAA, cluster
lights, bindless heap, texture-compression fallback, rasterizer
oracle regression criteria, frame-pacing CI gate, pak overlay
composition), the owned wgpu wrapper (`engine-gpu`), the Phase 5
PR slicing plan (6 PRs), the cargo-semver-checks adoption, the
acknowledged-deviations register, and the weekly reproducible-
build verification. CI gains a `wgpu::` grep guard, a
`cargo-semver-checks` step, and a scheduled reproducible-build
workflow as part of the audit's remediation.

The upper layers (render, physics, audio, net, ai, editor, hub, ui,
api, plugin-api) remain stubs and are built across the later phases.
Phase 5 (RENDERING FOUNDATION, Track A — deferred PBR, software
rasterizer oracle, RX 580 @ 60 FPS @ 1440p milestone) is next on
deck — see spec Part XXI and ADR-053.
