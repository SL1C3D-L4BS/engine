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
  `self-hosted-gpu` runner, evaluates p99 + σ against
  `tools/frame-pacing/budgets.toml`, and uploads the JSON report as
  a workflow artifact. The job is **informational** (continue-on-
  error) until the self-hosted GPU runner is provisioned per
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

**Engine Core v0.3 (Phase 5.5 — Track A GPU binding closure) — closed
2026-05-28.** A pre-Track-A audit (2026-05-27) reconciled the engine's
prior "Phase 6" naming with the spec: the in-progress work was
GPU-binding closure for spec Phase 5 (deferred PBR on RX 580
milestone), not the spec's true Phase 6 (3DGS + neural rendering).
v0.3 ships:

- **ADR-069 / ADR-074 / ADR-075** — the phase reconciliation, the
  Polaris-targeted `wgpu/vulkan` backend activation, and the per-pass
  `record()` discipline.
- **All 10 Track-A pass `record()` bodies wired** per ADR-075 §1
  (CullPass, CsmShadowPass, ClusterLightPass, GBufferPass, SsaoPass,
  IblPass, LightingAccumulationPass, TaaPass, BloomPass, TonemapPass);
  BloomPass dispatches the full 5-mip extract + 4-down + 4-up chain;
  CSM + GBuffer consume CullPass output via
  `multi_draw_indexed_indirect_count`.
- **Six pixel-parity oracle fixtures** under
  `crates/engine-render/tests/pixel_parity/` cross-validate the GPU
  path against the CPU oracle per ADR-046: `cube`, `csm_4_cascade`,
  `cluster_64_lights`, `ibl_probe`, `taa_motion`, `post_fx_chain`.
  Slice 8 of the cube fixture closed five engine-side formula
  divergences (reverse-Z, ClusterUniforms padding, directional
  lighting branch, GGX α-parameterisation, Narkowicz ACES) — the cube
  fixture now sits at `max_delta 0.0055 linear` (bit-accurate within
  the documented exception band in `docs/audit/oracle-exceptions.md`).
- **Upscaler cascade selects vendor.fsr** on every host the engine
  targets (ADR-076). `OwnedOnnxTemporal::supports()` returns true
  universally per the CPU-fallback contract; ADR-051 deviation entry 4
  moves from *scaffold* to *active*.
- **ADR-070 frame-pacing re-baseline** on the developer's actual
  hardware (Skylake 4c/8t + RX 580 + Mesa 26.1.1); GitHub-Actions
  `frame_pacing` runner removed in favour of the `just frame-pacing`
  local recipe.

**Engine Core v0.4 (Phase 6 — Neural Rendering & Gaussian Splatting) —
closed 2026-05-28.** Spec Phase 6 (3DGS + neural rendering + working
vendor cascade) shipped across eight PRs:

- **Pre-Phase-6 design sweep.** ADRs 077–084 lock the 3DGS
  architecture (`engine-splatting`), the ESPL asset format + glTF
  `KHR_gaussian_splatting` reader, the vendor SDK FFI discipline, the
  ONNX v1 training pipeline, the oracle exception sunset + ADR-046
  amendment (architectural-divergence category), the `engine-config`
  Level-1 crate, the `UpscalePass::record()` wiring + in-tree EASU
  shader, and the Phase 6 PR slicing record.
- **Oracle closures + new WGSL shaders.** `cluster_64_lights` closes
  to strict 1/255 (the lighting shader's point-light attenuation now
  matches the CPU oracle's windowed inverse-square kernel).
  `post_fx_chain` converts to a permanent architectural-divergence
  exception per ADR-046 §6a / ADR-081. Two new WGSL shaders land:
  `fsr_easu.wgsl` (Polaris-compatible EASU spatial path; closes the
  ADR-076 step-2 follow-up) and `bilinear_upscale.wgsl` (GPU 2×
  bilinear; replaces the documented CPU-oracle delegation).
- **`engine-splatting` Level-2 crate.** SplatCloud SoA storage, CPU
  radix sort by camera-space depth, ESPL asset format encode/decode,
  glTF `KHR_gaussian_splatting` reader, splat-composite pass
  contract. Two new WGSL shaders (`splat_sort.wgsl`,
  `splat_composite.wgsl`) ship with the renderer. 22 crate-level
  tests passing.
- **Vendor SDK FFI scaffold.** Three `*-sys` crates under
  `tools/upscaler-vendor-sdks/{streamline,fsr,xess}/` ready for SDK
  fetch per the new runbook (`docs/runbooks/vendor-upscaler-sdks.md`).
  ADR-051 deviation entries 5, 6, 7 acknowledge DLSS Streamline /
  FSR 4 / XeSS 2.
- **ONNX v1 training pipeline.** `tools/onnx-train/` ships pinned
  `requirements.txt` + `gen_training_data.py` + CNN+temporal+sub-
  pixel-conv `model.py` + `train.py` + `export.py` + `validate_ssim.py`.
  ADR-067 amendment 3 documents the v1 ship + ROCm explicit-disable
  on Polaris GFX8 + the achieved-SSIM clause. The actual training
  run is a user-runnable Python step that takes hours; the runtime
  loads the pre-trained `temporal_upscaler_v1.onnx` artifact.
- **`engine-config` Level-1 crate.** Owned line-oriented TOML reader
  consolidating the three previously-duplicated parsers per ADR-082.
  13 tests passing. Public surface: `Config`, `Section`, `Value`,
  `parse()`, quote-aware `strip_comment()` + `unquote()` helpers.
- **3DGS frame-pacing scenes.** `splat_garden_1m.ron` (the spec
  line-1636 milestone — 1M-splat scene > 60 FPS) +
  `combined_pbr_plus_splat.ron` (full graph + 100k-splat overlay).
  `tools/frame-pacing/budgets.toml` grows per-scene override rows.
  The actual measurement runs locally on the user's RX 580 per
  ADR-070.

`engine.toml` now reads `phase = "6-closed"`. On the user's RX 580
the cascade selects `vendor.fsr` (EASU spatial path) per ADR-076; on
tier-appropriate hardware (RTX 40+, Arc B+) the cascade extends to
the vendor SDK paths once the runner runs `cargo build --features
all-vendors`. The owned ONNX temporal upscaler runs on the CPU AVX2
backend on Polaris (ROCm explicitly disabled per ADR-080).



Phase 6 (renamed to Phase 5.5 per ADR-069) contract-side closed
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

**Sub-PRs landed 2026-05-27** that complete the shader-source +
crate-scaffold surface in advance of the runner-gated binding work:

- **PR 3.5 — WGSL shaders for geometry + lighting** (ADR-064). Five
  shaders under `crates/engine-render/shaders/` (`cull.wgsl`,
  `csm_shadow.wgsl`, `gbuffer.wgsl`, `cluster_assign.wgsl`,
  `lighting.wgsl`) implementing the cull / CSM / G-buffer /
  cluster-assign / Cook-Torrance algorithms. Cross-checked against
  the CPU oracle's constants (32 lights/cluster, 16×9 workgroup
  size, etc.) in unit tests. Embedded into engine-render via
  `include_str!` in `src/shaders.rs`.
- **PR 4.5 — WGSL shaders for post-FX** (ADR-065). Six shaders
  (`ssao.wgsl`, `brdf_lut_bake.wgsl`, `ibl_evaluate.wgsl`,
  `taa_resolve.wgsl`, `bloom.wgsl` with three entry points,
  `tonemap.wgsl`) implementing 8-tap Fibonacci SSAO,
  Hammersley + GGX-importance BRDF LUT bake, L2 SH evaluation
  (Ramamoorthi-Hanrahan 2001), YCgCo neighbourhood-clip TAA,
  soft-knee bloom chain, ACES filmic (Stephen Hill fit) tonemap.
- **PR 5.5 — `crates/engine-upscale-vendor/` crate scaffold**
  (ADR-066 §1). New Level-1 crate with per-vendor module skeletons
  gated by cargo features (`dlss`, `fsr`, `xess`, `ort-runtime`,
  `all-vendors`) — all default off so CI without SDKs builds
  unchanged. `build_info()` const-fn surfaces feature flags into
  bench reports. ADR-051 gets deviation entry 4 anticipating the
  ORT integration. `engine.toml` `[upscaler]` schema documented.

**PR 7 — GPU pipeline binding (engineering-only, landed
2026-05-27).** Per a two-PR split of the original v0.3 candidate
(ADR-068 third addendum), PR 7 lands the pure-Rust work that does
not need vendor SDKs, an ONNX model, or a GPU runner:

- `PassContext` extension with `GpuFrameContext { device, encoder }`
  so `Pass::record` bodies bind against `engine_gpu` directly
  (`crates/engine-render/src/render_graph.rs`).
- Per-pass `std::sync::OnceLock` pipeline fields + `pub fn new(...)`
  constructors on every Track-A pass struct; compute-pass `record()`
  bodies open a `ComputePass`, set the lazy-built pipeline, and
  issue placeholder dispatches (real dispatch counts come with PR 8's
  resource lookups). Render-pass `record()` bodies lazy-init only
  pending PR 8's attachment-view plumbing.
- `wgsl_artefact_set(stage, entry, source) -> ShaderArtefactSet` — a
  raw-WGSL bridge so the 11 hand-written shader sources flow through
  the existing `build_{render,compute}_pipeline` helpers without a
  `slangc` round-trip.
- New `engine_render::init` module with `build_brdf_lut_bake_pipeline`
  (one-shot, init-time bake — deliberately not a per-frame `Pass`)
  and `build_all_phase6_pipelines(device) -> Phase6Pipelines`, the
  bundle a future PR-8 pixel-parity fixture pre-warms.
- `engine_render::upscaler_config` — owned line-oriented reader for
  the `engine.toml [upscaler]` section
  (`provider` ∈ {auto, dlss, fsr, xess, owned-onnx, owned-bilinear},
  `quality` ∈ {performance, balanced, quality, ultra-quality}).
  `UpscalerRegistry::with_phase6_defaults_from_config(&cfg)` registers
  the forced provider + `OwnedBilinear` fallback per ADR-066 §6.
- Integration smoke at `crates/engine-render/tests/pipeline_smoke.rs`
  (`#[ignore]` by default; runs against a fallback-adapter device
  when wgpu has a backend feature enabled).

Tests: 596 → 610. `just ci` green. ADR-049 wgpu boundary
preserved.

**PR 7.5 — code-review follow-up (engineering-only, landed
2026-05-27).** Closed 10 of the 15 findings surfaced by a multi-angle
code review against PR 7. The `Pass` trait gained
`install_pipeline(device)` so pipeline-build failures surface at
startup with `Result<(), ShaderError>` instead of panicking on the
per-frame hot path; per-pass `OnceLock` storage became `Option`;
`record()` bodies short-circuit on `gpu = None` or "pipeline not
installed", restoring the CPU-rasterizer no-op path. Per-frame
`.clone()` on `Arc`-wrapped pipelines removed. `GBufferPass` and
`BloomPass` outer labels aligned with `pass.name()`. The TOML reader
in `upscaler_config.rs` became quote-aware (preserves `#` inside
strings), rejects unbalanced quotes via a new `ParseError` variant,
and tolerates `[ upscaler ]` whitespace. `UpscaleCtx` gained
`quality: Quality` + a `Quality::scale() -> f32` divisor helper.
`with_phase6_defaults` delegates to `…_from_config(&Default::default())`
so the ADR-066 cascade order has one source of truth. The smoke test
no longer forces a fallback adapter (the `Device::new(_, true)` flag
is wgpu's `force_fallback_adapter`, not graceful fallback) and now
surfaces the failing pass name. Tests: 610 → 615 (+5). `just ci`
green.

The five remaining findings fold into PR 8 (real bind-group layouts,
real vertex-buffer layouts, smoke-test backend feature, frame-pacing
gate promotion) or follow-up cleanup (shared `engine-config` crate for
the TOML helpers duplicated across three sites).

**PR 8 — Engine Core v0.3 closure (deferred, runner-gated).** The
remaining work folds into a single PR that needs environmental
prerequisites (RDNA2-class runner + downloaded DLSS/FSR/XeSS SDKs +
ORT native binaries + Git LFS):

- Vendor SDK FFI: `*-sys` crates at
  `tools/upscaler-vendor-sdks/{streamline,fsr,xess}/`; per-vendor
  `Real` provider impls flipping `supports_stub()` to real
  `supports(device)` probes.
- ONNX integration: `ort` dep behind the `ort-runtime` feature;
  bundled `crates/engine-render/assets/onnx/temporal_upscaler_v1.onnx`
  via Git LFS; `OwnedOnnxTemporal::supports()` becomes a runtime
  probe; ADR-051 entry 4 flips from "anticipated" to "active".
- Six pixel-parity oracle fixtures (3 ADR-064: cube,
  csm_4_cascade, cluster_64_lights; 3 ADR-065: ibl_probe,
  taa_motion, post_fx_chain) rendering via both the CPU oracle
  (`testbed/engine-raster`) and the GPU path, gated by ADR-046's
  1/255 channel + p99 ≤ 1% threshold.
- Real `begin_render_pass` plumbing in the render-pass bodies
  (csm_shadow / gbuffer / lighting) plus real bind-group
  descriptors against the WGSL `@group/@binding` annotations.
- Frame-pacing CI gate promotion (ADR-047 §7): flip
  `.github/workflows/ci.yml` `frame_pacing` from
  `continue-on-error: true` to required when the runner is
  provisioned per `docs/runbooks/frame-pacing-runner.md`.
- `engine.toml` `phase = "6"` → `"6-closed"`; README v0.3 paragraph;
  ADR-068 final close addendum carrying the tag commit hash.
- `git tag v0.3` (post-merge user action).

`engine.toml` reads `phase = "6"`. The upper layers (physics, audio,
net, ai, editor, hub, ui, api, plugin-api) remain stubs and are built
across Phases 7+ per the spec's level and phase map.
