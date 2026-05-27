# [ENGINE] Platform

A Rust-first, zero-runtime-dependency game engine and platform.

This repository is the monorepo through **Phase 6 contract-side close**
(RENDERING
FOUNDATION, Track A — deferred PBR, software rasterizer oracle,
RX-580 @ 60 FPS @ 1440p milestone). The crate tree, levels, and
architecture follow the authoritative specification:

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

Phase 5 (RENDERING FOUNDATION, Track A) closed 2026-05-27 across six
PRs per ADR-053. The renderer is real: deferred PBR + cluster lights +
CSM + IBL + post-FX + upscaler trait surface, with the software
rasterizer testbed (`testbed/engine-raster/`) serving as the cross-
backend pixel-parity oracle (ADR-046).

- **PR 1 — Render-graph trait surface + rasterizer oracle.** The
  `engine_render::render_graph` trait surface (Pass, Resource,
  ResourceSet, RenderGraph, Track) lands alongside the substantive
  software rasterizer. Pixel-parity oracle, ADR-039 + ADR-046.
- **PR 2 — `engine-gpu` owned wgpu wrapper + bindless heap + BC
  import.** New Level-1 crate `engine-gpu` is the only crate
  permitted to name `wgpu::*` (ADR-049 boundary, CI grep guard).
  `BindlessHeap` (24-bit slot + 8-bit generation, LIFO free-list,
  ADR-044), `TextureMeta` BC{4,5,7} import header (ADR-045),
  `tools/engine-tex-compress/` runtime.
- **PR 3 — Deferred G-buffer + cluster lights + CSM.** The first
  visible image: deferred geometry pass with bindless-indexed slots,
  cull + cluster-lights compute (16×9×24 grid, 32 lights/cluster cap),
  CSM (practical-split λ=0.6, 4096² atlas, Vogel-disk 16-tap PCF),
  lighting accumulation (Cook-Torrance + cluster + CSM). ADR-040 +
  ADR-043.
- **PR 4 — IBL L2 SH probes + post-FX chain.** IBL (128 probes,
  Karis split-sum), SSAO, bloom, ACES tonemap, TAA (Halton 2,3
  period-8 jitter, YCgCo neighbourhood-clip rejection, motion-vector
  reprojection). The full 10-pass Track-A schedule pinned. ADR-041 +
  ADR-042.
- **PR 5 — UpscalerProvider trait + RX-580 milestone bench.** Four
  upscalers ship: DLSS / FSR / XeSS as `supports() = false` vendor
  stubs (real bindings Phase 6), `OwnedBilinear` placeholder. The
  ADR-005 selection cascade (vendor > best match > owned) is
  exercised by `crates/engine-render/tests/upscale_selection.rs`.
  `bin/engine-bench-frame-pacing/` ships the milestone harness with
  an owned arg parser, owned JSON report, owned TOML budgets reader,
  and a deterministic CPU oracle workload (the
  `combined_deferred_scene` full pipeline + bilinear upscale). ADR-
  005 + ADR-053 §PR-5.
- **PR 6 — Frame-pacing CI gate + Phase 5 closure.** The
  `frame_pacing` job lands in `.github/workflows/ci.yml`, runs on a
  `self-hosted-gpu-rx6700xt` runner, evaluates p99 + σ against
  `tools/frame-pacing/budgets.toml`, and uploads the JSON report as
  a workflow artifact. The job is **informational** (continue-on-
  error) until the self-hosted RX 6700 XT runner is provisioned per
  `docs/runbooks/frame-pacing-runner.md`; the promotion path is
  documented in the runbook. ADR-016 + ADR-047 + ADR-053 §PR-6.

PR 6 also closes two audit follow-ups:

- **Full ADR-060 sli aggregate-ops surface.** The 12 opcodes
  0x70-0x7B (ArrayNew/Get/Set/Len, MapNew/Get/Set, StructNew/Get/Set,
  ClosureMake, CallClosure) landed pre-Phase-5 alongside the
  hand-assembled oracle; PR 6 wires codegen: AST + parse + typeck
  for `[1, 2, 3]` array literals and `[k => v, k => v]` map literals,
  plus codegen for `Field`, `Index`, `StructLit`, and `Closure`
  (free-variable discovery + capture-list emission). End-to-end
  `tests/aggregate_ops_codegen.rs` exercises source-through-VM.
- **Missing ADR-048 pak overlay tests.** Two follow-up tests
  (`unmount_handle_drops_overlay_assets`,
  `dedupe_refcount_does_not_double_free`) close the §Verification
  surface from the audit.

**Engine Core v0.2** is tagged at the close of Phase 5.

Phase 6 (RENDERING FOUNDATION, Track A, Part 2) contract-side closed
2026-05-27 across the ADR-068 six-PR slicing:

- **Pre-Phase-6 design sweep.** ADRs 061–068 lock the GPU pass
  contracts, the vendor upscaler binding discipline, the owned ONNX
  upscaler, the mesh + material owned formats, the glTF importer
  subprocess, and the shader-to-pipeline binding surface (eight ADRs,
  2 278 lines). No code; pre-Phase-6 design lockdown.
- **PR 1 — Mesh + material asset formats + glTF importer subprocess.**
  `engine-asset` gains `MeshMeta` (`EMSH`, 24-byte header per
  ADR-061 §1) and `MaterialMeta` (`EMAT`, 24-byte header per
  ADR-061 §2). `tools/engine-mesh-import/` wraps the `gltf` 1.4
  crate as a subprocess CLI per ADR-062: owned arg parser, owned
  JSON manifest, typed exit codes for parser-crash / schema-invalid
  / unsupported / io failures, smoke + determinism + red-team
  coverage. New CI grep guard rejects `gltf::` outside the importer
  directory, mirroring ADR-049's wgpu boundary.
- **PR 2 — Shader artefact ingest + pipeline-construction wiring.**
  `engine-render::shader` lands `ShaderArtefactSet` +
  `build_render_pipeline` + `build_compute_pipeline` per ADR-063.
  Selects the WGSL artefact (the canonical wgpu-backed engine_gpu
  target); widens to SPIR-V/DXIL/MSL when native backends ship.
- **PR 3 — GPU pass contracts (geometry / lighting / post-FX).**
  `engine-render::contracts` pins the ADR-064 + ADR-065 surface
  in Rust: `PushConstants` (64 B), `CsmUniforms`, `ClusterUniforms`
  + `ClusterCell` + `LightRecord` SSBO records, MRT format
  constants, SSAO / IBL / TAA / Bloom / Tonemap uniform structs,
  `IblProbeRecord`. `testbed/engine-raster/tests/rendering_contracts.rs`
  cross-checks every grid + cascade + cap constant against the CPU
  oracle's source-of-truth values so a future shader edit can't
  silently desync.
- **PR 5 — OwnedOnnxTemporal cascade reservation.** The trait
  discriminant Phase 5 PR 5 reserved (`UpscalerKind::OwnedOnnx`)
  gains its provider impl. `UpscalerRegistry::with_phase6_defaults()`
  registers DLSS → FSR → XeSS → OwnedOnnxTemporal → OwnedBilinear
  per ADR-066 §6 priority. `with_phase5_defaults()` deprecated.

**Deferred to follow-up PRs** that require the self-hosted RX 6700 XT
runner + vendor SDK downloads + `ort` + Git LFS setup:

- **Phase 6 PR 3.5 / 4.5 — GPU `record()` body implementations**
  (CullPass / CsmShadowPass / GBufferPass / ClusterLightPass /
  LightingAccumulationPass + SsaoPass / IblPass / TaaPass / BloomPass
  / TonemapPass) + Slang shader sources (`crates/engine-render/
  shaders/*.slang`) + 3 + 3 pixel-parity oracle fixtures per
  ADR-064 / ADR-065 §Verification. Pixel parity cannot be validated
  in CI without a GPU; the runner is the missing piece.
- **Phase 6 PR 5.5 — Vendor upscaler FFI + ONNX integration.** The
  `crates/engine-upscale-vendor/` crate split per ADR-066 §1; the
  vendored `tools/upscaler-vendor-sdks/{streamline,fsr,xess,ort}/`
  `*-sys` crates; the bundled `temporal_upscaler_v1.onnx` model via
  Git LFS; the `engine.toml` `[upscaler]` runtime reader; the
  ADR-051 amendment adding the ORT deviation entry per ADR-067 §5.
- **Phase 6 PR 6.5 — Frame-pacing gate promotion.** Flip
  `.github/workflows/ci.yml` `frame_pacing` job from
  `continue-on-error: true` to required (ADR-047 §7) once the
  runner is provisioned per
  `docs/runbooks/frame-pacing-runner.md` and the first green
  baseline lands.
- **Engine Core v0.3 tag** ships when the runner-gated deliverables
  above complete.

`engine.toml` reads `phase = "6"` to mark the contract-side closure.
The upper layers (physics, audio, net, ai, editor, hub, ui, api,
plugin-api) remain stubs and are built across Phases 7+ per the
spec's level and phase map.
