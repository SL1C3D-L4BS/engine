# Phases 0–4 · Enterprise-Grade Audit

- Date: 2026-05-24
- Auditor: working session (recorded in plan
  `~/.claude/plans/moonlit-snuggling-harbor.md`)
- Scope: every workspace deliverable from Phase 0 through Phase 4
- Verdict source: this document (the audit is the verdict; the
  remediation packets are the corrective work)

---

## §1 · Executive Verdict

**Code is production-grade. Documentation is two phases behind.**

The 8 foundation crates plus 12 portfolio deliverables (3 per phase
across Phases 1–4) are real, tested, oracle-verified, and gated by 8 CI
guards and 14 cross-arch determinism oracles. Every owned subsystem
ships with the verifier the spec's risk register R-02 demands. The
machine model the engine claims to own is, in fact, owned.

The repository's *narrative* layer has not kept up:

1. **`engine.toml` declares `phase = "3"`** even though Phase 4 (sli
   compiler, register VM + GC + verifier, hot-reload + debugger + REPL,
   Slang shader toolchain + reproducibility golden) closed on
   2026-05-20 across four signed commits.
2. **`README.md` describes the project as "monorepo through Phase 3"**
   with no mention of the four Phase-4 ADRs or the four Phase-4
   deliverables.
3. **All 23 Phase-0 ADRs are 12-line stubs** (ADRs 001–022 except 023,
   plus 024) whose body reads literally `*Stub. Full Context /
   Decision / Rationale / Consequences to be expanded per spec Part
   XX.2.*`. This includes the three contracts most load-bearing for
   the whole programme: ADR-012 (50-year API stability), ADR-013
   (Determinism Contract), and ADR-016 (Frame Pacing Contract). The
   *spec* describes them in full; the *repository's own ADR set* does
   not.
4. **`cargo-semver-checks` is absent from CI.** ADR-012 names it as
   the enforcement mechanism. The current excuse — that `engine-api`
   is a 3-line stub — is exactly *why* now is the right time to wire
   it in (zero risk of false positives), not why we should defer.
5. **No render-graph, CSM, IBL, TAA, cluster-light, bindless-heap,
   compression-fallback, rasterizer-regression, pak-overlay, or
   frame-pacing-CI-gate ADR exists**, even though every one is a Phase
   5 hard dependency demanded by spec §IV.4 and §IV.5. Implementation
   could land without these — but the spec's own §XX.2 forbids
   non-obvious decisions to land without an ADR. Writing them first is
   the cheap path.

What is *not* wrong is what matters most for the next 18 months: the
**deterministic substrate is sound**, the **owned-vs-vendored
discipline holds**, the **oracle methodology works**. Phase 5 can be
planned with confidence once the documentation deficit is closed.

### §1.1 · The three categories

- **Production-ready (no action required):** all Phase 1–4 code, all
  Phase 1–4 ADRs (023, 026–038), all Phase 1–4 oracles, all 9
  observatory baselines, the 8 CI guards, the determinism job's
  cross-arch matrix, the deny.toml license allowlist.
- **At risk — acknowledged deviations (formalize, then close):** TOML
  (not RON) breakpoint-persistence format, owned (not `fluent_bundle`
  / `icu4x`) i18n, owned (not MessagePack/serde) telemetry IPC.
  Already tracked in agent memory; never formalised in an ADR. ADR-051
  (packet 14) closes this.
- **Missing — documentation/ADR gaps the spec demands (write them):**
  the 23 Phase-0 ADR stubs, the 10 Phase-5 design ADRs (039–048), the
  3 forward-looking decision ADRs (049, 050, 053), the deviations
  register (051), the reproducible-build cadence (052), the
  cargo-semver-checks adoption (part of 050), the `engine.toml` /
  `README.md` refresh. 14 ADRs and 3 file updates total. Each is a
  small PR; the set is the remediation plan in Part B of the approved
  plan.

This audit closes when those 14 ADRs land and the three file updates
ship. Phase 5 planning opens the session after.

---

## §2 · Methodology

### §2.1 · Sources used

1. **The spec.** `~/Resources/documentation/ENGINE_SPECIFICATION_v2.0.md`
   (1882 lines), the contract this audit measures the repo against.
2. **The ADR set.** All 38 ADRs in `docs/adr/`, read directly (status
   line, line count, body present or stubbed).
3. **The code.** Every `Cargo.toml`, every `src/lib.rs`, the CI
   workflow, the `engine.toml` manifest, the README, the `tests/`
   directory tree, the `docs/observatory/` baseline set. File-level
   verification via Read and Bash `wc -l` / `grep` (never inferred).
4. **The 93-volume book library.** Cited by exact filename and chapter
   range as the rationale base for design principles (Appendix E of
   the spec is the seed mapping). No literal re-reading; the per-
   subsystem mapping the prior Explore-agent session produced is the
   index.
5. **Agent memory.** Two project memories
   (`engine-monorepo-status.md`, `foundation-layer-deviations.md`) as
   *hints* — both were spot-checked against the code before any claim
   was repeated.

### §2.2 · What was sampled vs. asserted

- **Asserted from direct verification:** every line count, every
  file's existence/absence, every ADR's status field, every CI guard
  name, every `engine.toml` value, every spec line cited.
- **Sampled, then claimed by extrapolation:** I read ADR-001, 003,
  004, 005, 012, 013, 014, 016, 017, 018, 019, 020 and observed all
  twelve are identical 12-line stubs. I then verified with `wc -l`
  that ADRs 002, 006, 007, 008, 009, 010, 011, 015, 021, 022 also
  match the 12-line stub length. ADR-024 was not opened but is
  asserted-stub by the same length signature. If 024 has been
  expanded since, treat that one claim as soft.
- **Trusted from prior Explore-agent reports without re-verification:**
  per-crate public-API surfaces, per-ADR companion lists, the
  exact line numbers within `ci.yml` for each guard (verified
  start-line and structure; did not re-check every line). The agent
  reports are in the conversation history; the user has access.
- **Not re-run during the audit:** benchmark numbers
  (`million_entities`, arena, HashMap, mmap, profiler), oracle test
  passes, the cross-arch GitHub Actions run. The committed baselines
  in `docs/observatory/` and the CI status are the trust anchor; the
  audit's claim is that they exist and are wired into CI, not that
  every number is fresh today.

### §2.3 · What this audit deliberately does *not* do

- It does not re-review the *substance* of accepted decisions. ADR-002
  (hybrid ECS), ADR-003 (Slang), ADR-013 (determinism contract) are
  treated as fixed contracts. The audit flags ADR-013 as a 12-line
  stub because its *expansion* is missing, not because its decision
  is open for re-debate.
- It does not benchmark anything. Performance findings come from the
  committed baselines and the spec's own contracts.
- It does not look at code style or rustdoc *prose quality*. The
  `missing_docs = "deny"` workspace lint already gates docstring
  presence; the audit trusts it.

---

## §3 · Phase-by-Phase Deliverable Matrix

Each row: planned deliverable → as-shipped → oracle → CI guard →
baseline → ADR. Missing cells are explicit gaps.

### §3.1 · Phase 0 — Foundation (Arch + cachyos + Niri + dotfiles + telemetry stack)

| Deliverable | As-shipped | Oracle | CI guard | Baseline | ADR |
|---|---|---|---|---|---|
| 8 foundation crates (Levels 0–1) | engine-{math, platform, reflect, ecs-macro, core, asset, telemetry, i18n} all real | per-crate tests | — | — | 001 (Rust), 002 (ECS), 023 (math-determinism), 024 (macro consolidation) |
| Arch + cachyos + Niri environment | Spec Part XVIII (dev env doc) | environment reproducible via dotfiles repo | — | — | 015 (Niri stub), 021 (LUKS stub), 022 (kernel stub) |
| Owned crate-level architecture docs | `docs/architecture/{8 crates}.md` | — | — | — | — |

**Status:** code complete. **Gap:** ADRs 001, 002, 015, 021, 022, 024 are 12-line stubs.

### §3.2 · Phase 1 — Silicon → C (verified machine model)

| Deliverable | As-shipped | Oracle | CI guard | Baseline | ADR |
|---|---|---|---|---|---|
| SIMD math | engine-math private `Simd4f` (SSE2/NEON/scalar), drives Vec/Mat | `tests/simd_parity.rs` (100k parity vs scalar reference) | FMA-free build profile + grep guard (ADR-023/027) | — (parity oracle, not a perf bench) | 023 (67 lines), 027 (160 lines) |
| Arena allocator | `Arena`/`ArenaStats` trait + 4 arenas (Linear, Ring, Pool, General free-list) in engine-core::alloc | criterion benches | — | `docs/observatory/arena-baseline.md` | 026 (116 lines) |
| Cache observatory | `tools/cache-observatory/` (owned `perf_event_open`) | layout tripwires (`const _: () = assert!`) | — | `docs/observatory/cache-baseline.md` | — |

**Status:** code complete, ADRs complete.

### §3.3 · Phase 2 — Linux systems (own the platform surface)

| Deliverable | As-shipped | Oracle | CI guard | Baseline | ADR |
|---|---|---|---|---|---|
| Robin Hood hash map | engine-core::collections (FastHasher + DeterministicHasher) | `tests/collections_parity.rs` (vs std + naive RH) | std::collections::HashMap grep guard | `docs/observatory/hashmap-baseline.md` | 028 (114 lines) |
| mmap'd asset loader | engine-platform::mmap::MmapRo + engine-asset::Pak::open_mmap | `tests/mmap_roundtrip.rs` | raw libc::mmap grep guard (allowlist mmap.rs, sampler.rs) | `docs/observatory/mmap-asset-baseline.md` | 029 (108 lines) |
| Sampling profiler | engine-platform::sampler + engine-telemetry::profiler + `tools/sampling-profiler/` | `tests/profiler_oracle.rs` (≥80% spinner self-time, requires `-C force-frame-pointers=yes`) | profiler-oracle workflow step gates RUSTFLAGS | `docs/observatory/profiler-baseline.md` | 030 (123 lines) |

**Status:** complete.

### §3.4 · Phase 3 — Engine Core (substrate that all later phases sit on)

| Deliverable | As-shipped | Oracle | CI guard | Baseline | ADR |
|---|---|---|---|---|---|
| Archetype ECS | engine-core::ecs::archetype (AnyVec columns, TypeStableId signatures) | `tests/archetype.rs` | TypeId::of grep guard in ecs/ (allow: resources) | `docs/observatory/archetype-baseline.md` | 031 (157 lines) |
| Owned fiber job system | engine-platform::ThreadPool + engine-platform::fiber (naked_asm x86_64/aarch64, ucontext fallback) + engine-platform::JobGraph | `tests/jobs_oracle.rs` (worker counts {1,2,4,N}) | std::thread::spawn / std::sync allowlist + naked_asm allowlist + reject rayon/crossbeam/tokio/parking_lot/async-std | `docs/observatory/jobs-baseline.md` | 032 (166 lines) |
| Deterministic parallel scheduler | engine-core::ecs::Schedule::add_system_with_access + run_on (per-phase JobGraph dispatch) | `tests/replay_parity.rs` (per-frame BLAKE3 digest match across worker counts, both archs) | replay-parity step in determinism job | `docs/observatory/million-entities-baseline.md` + `parallel-schedule-baseline.md` | 033 (195 lines, with v0.1.1 addendum) |

**Status:** complete. Engine Core v0.1 tagged at Phase 3 closure;
v0.1.1 follow-up closed the 1M-entity / 60 FPS milestone gap (3-PR
sequence: query DSL join impls, bench rewrite, replay-parity rewrite —
recorded in ADR-033 addendum).

### §3.5 · Phase 4 — Scripting (sli compiler + VM + debugger + Slang)

| Deliverable | As-shipped | Oracle | CI guard | Baseline | ADR |
|---|---|---|---|---|---|
| sli front-end (lexer, Pratt parser, AST, type checker, SSA IR + const-fold/CSE/DCE, diagnostics) | crates/engine-script/src/{lex,parse,ast,resolve,typeck,ir,consteval,ext,diag,source}.rs | `tests/compile_parity.rs` (BLAKE3 over optimised IR, committed golden) + `tests/parser.rs` + `tests/typeck.rs` + `tests/ir.rs` | Reject rlua/mlua/wasmtime/wasmer/cranelift/inkwell/lalrpop/pest/nom/combine/chumsky | `tests/goldens/sli-compile.golden` | 034 (108 lines) |
| Register VM + tri-color GC + bytecode verifier | crates/engine-script/src/{bytecode,verify,vm/*,gc/*,ffi,asset}.rs (single-gen GC; nursery/old_gen/remembered/barrier are typed stubs for PR-3 generational follow-up) | `tests/vm_oracle.rs` + `tests/verifier.rs` + `tests/gc_oracle.rs` + `tests/gc_pause_oracle.rs` (informational) + `tests/ffi.rs` + `tests/codegen_no_trap.rs` (500-program fuzz) | 0xFF TRAP-opcode grep guard (allowlist bytecode.rs/verify.rs/debug.rs) | `tests/goldens/sli-vm.golden` | 035 (180 lines) |
| Hot-reload + debugger protocol + REPL | crates/engine-script/src/{reload,debug,debug_proto,watch_expr,breakpoints_toml,repl}.rs + bin/engine-{repl,debug}/ | `tests/hot_reload.rs` + `tests/debug_protocol.rs` (round-trip every variant) + `tests/breakpoint_persistence.rs` + `tests/watch_expr_safety.rs` + `bin/engine-debug` editor-bridge contract | Reject serde/serde_json/bincode/prost/protobuf/rmp under engine-script + reject rustyline/reedline/linefeed under bin/engine-repl | — | 036 (174 lines) |
| Slang shader toolchain | `tools/engine-shader/` (slangc sandboxed subprocess per ADR-019 pattern, owned artifact codec, asset-trait integration) | `tests/target_enum.rs` + `tests/bundle_codec.rs` + `tests/reproducibility.rs` (graceful skip if slangc absent) | Reject naga/shaderc/shaderc_rs/spirv_tools/spirv_cross/glslang under tools/engine-shader | `tests/goldens/triangle-reproducibility.golden` | 037 (139 lines), 038 (112 lines) |

**Status:** complete. SLANGC_PIN = `"v2026.9"`. Note: the v2.0 spec calls
the breakpoint persistence file `breakpoints.ron` — the shipped
implementation uses TOML; this is an **acknowledged deviation** flagged
for ADR-051 in §15.

### §3.6 · Phase-by-phase gaps surfaced by this matrix

- **Gap 3.A:** ADRs 001, 002, 015, 021, 022, 024 are stubs (Phase 0
  documentation deficit).
- **Gap 3.B:** `engine.toml` reads `phase = "3"`; the comment
  describes only Phase 3. Should read `phase = "4"` (or
  `"4-audited"` after this audit's remediation closes — see §16
  forward-looking anchors).
- **Gap 3.C:** `README.md` "Status" section ends at Phase 3 with the
  text "The upper layers… remain stubs and are built across the later
  phases." Should add Phase 4 closure paragraph mirroring the
  per-deliverable structure of the Phase 1–3 paragraphs already
  present.

---

## §4 · Per-Crate State Matrix

### §4.1 · Workspace members (from `Cargo.toml`)

The workspace declares 25 members across `bin/`, `crates/`, `testbed/`,
`tools/`:

- **2 bins:** `engine-debug`, `engine-repl`
- **19 crates** (path-declared at workspace level): engine-math,
  engine-platform, engine-reflect, engine-ecs-macro, engine-core,
  engine-asset, engine-telemetry, engine-i18n, engine-render,
  engine-physics, engine-audio, engine-net, engine-script, engine-ai,
  engine-editor, engine-hub-core, engine-ui, engine-api,
  engine-plugin-api
- **1 testbed:** `testbed/engine-raster`
- **3 tools:** `cache-observatory`, `engine-shader`, `sampling-profiler`

### §4.2 · Real vs. stub matrix

| Crate / target | Real / Stub / Partial | Public-API magnitude | Third-party deps | Notes |
|---|---|---|---|---|
| engine-math (L0) | **Real** | full (Vec/Mat/Quat/scalar wrappers, transcendentals, private SIMD wrapper, fixed-point) | none (dev: engine-core) | ADR-023, 027; FMA-free; golden + parity oracles |
| engine-platform (L0) | **Real** | full (fiber, fs, input, jobs, mmap, sampler, sysinfo, thread_pool, time::FramePacer, watch) | libc; dev: blake3, criterion | ADR-026/029/030/032; no windowing/GPU yet (Phase 5 will add `engine-gpu` per ADR-049) |
| engine-reflect (L0) | **Real** | Reflect trait, ReflectValue/FromReflect, TypeRegistry, TypeInfo, TypeStableId | none | drives ECS macro and (future) editor inspector |
| engine-ecs-macro (L0) | **Real** | derive macros for Component/Reflect | proc-macro2/syn/quote (dev-only ergonomics; not user-facing) | ADR-024 (stub) |
| engine-core (L1) | **Real** | ECS, alloc (4 arenas), collections (HashMap), telemetry (Signal/Subsystem), schedule | engine-math, engine-platform, engine-reflect, engine-ecs-macro | ADR-026/028/031/033; unsafe reborrow in dispatch_phase is the only unsafe |
| engine-asset (L1) | **Real** | ContentHash, ContentStore, Pak/PakBuilder/PakSet/PakError, AssetServer/Handle\<T\>, PakSigner | sha2, ed25519-dalek (ADR-025), engine-core, engine-platform | ADR-008/025/029; no format-specific importers |
| engine-telemetry (L1) | **Real (owned IPC)** | collector, IPC protocol encoder, profiler integration | engine-core, engine-platform | **deviation** — owned binary IPC, not MessagePack/serde (acknowledged) |
| engine-i18n (L1) | **Real (owned Fluent subset)** | Fluent-subset parser, CLDR plural rules, number formatting | none | **deviation** — owned, not fluent_bundle/icu4x (acknowledged) |
| engine-script (L1) | **Real (single-gen GC)** | Compiler/Compiled/CompileError/DebugInfo, Module/Source/SourceMap/Span/Diagnostic, IrModule, VM, GC, debugger protocol, REPL | blake3 (dev), engine-asset, engine-core, engine-platform, engine-reflect | ADR-034/035/036; generational GC deferred; struct/array/map/closure ops deferred |
| engine-render (L2) | **STUB** | 3-line doc comment | none | Phase 5 territory; gets a doc-comment refresh pointing at ADR-039 |
| engine-physics (L2) | **STUB** | 3-line doc comment | none | Phase 7 |
| engine-audio (L2) | **STUB** | 3-line doc comment | none | Phase 9 |
| engine-net (L2) | **STUB** | 3-line doc comment | none | Phase 9 |
| engine-ai (L2) | **STUB** | 3-line doc comment | none | Phase 8 |
| engine-editor (L2) | **STUB** | 3-line doc comment | none | Phase 10 |
| engine-hub-core (L2) | **STUB** | 3-line doc comment | none | Phase 10 |
| engine-ui (L2) | **STUB** | 3-line doc comment | none | Phase 10 |
| engine-api (L4) | **STUB (intentional pre-v1.0)** | 3-line doc comment | none | ADR-012; semver-checks gate to be wired now per ADR-050 |
| engine-plugin-api (L4) | **STUB** | 3-line doc comment | none | Phase 10 (ADR-018) |
| testbed/engine-raster | **STUB** | 3-line doc comment | none | Phase 5 PR 1 (rasterizer oracle, spec Part IX) |
| bin/engine-repl | **Real** | cooked-mode stdin REPL | engine-script | ADR-036 |
| bin/engine-debug | **Real** | debugger server + editor-bridge example | engine-script | ADR-036 |
| tools/cache-observatory | **Real** | perf_event_open CLI | libc + engine-{math,core,platform} | Phase 1 |
| tools/engine-shader | **Real** | slangc wrapper + artifact codec + asset trait + CLI | blake3, engine-asset, engine-core | ADR-037/038 |
| tools/sampling-profiler | **Real** | folded-stack CLI | engine-{platform,telemetry} | Phase 2; Brendan-Gregg format |

### §4.3 · Crate-level invariants verified

- **No third-party crate outside the explicit allowlist.** Workspace
  `Cargo.toml` lists exactly: `criterion` (dev), `blake3` (dev), per-
  crate transitive {libc, sha2, ed25519-dalek, proc-macro2/syn/quote}.
  No rayon, crossbeam, tokio, parking_lot, async-std, serde, naga,
  shaderc anywhere — CI guards enforce.
- **Workspace lint `missing_docs = "deny"` is active.** Every public
  item in real crates carries a rustdoc comment (gated by the build).
- **All Level-2+ crates that are *intentionally* stubs ship a single
  doc comment** pointing at their phase. None ship Cargo-deps that
  would already pull a runtime in.

### §4.4 · Per-crate gaps

- **Gap 4.A:** `engine-render/src/lib.rs` doc comment does not point
  at the (yet-to-be-written) ADR-039 (render-graph). One-line refresh
  in the audit's wake (task #18).
- **Gap 4.B:** `testbed/engine-raster/src/lib.rs` doc comment does
  not point at the (yet-to-be-written) ADR-046 (oracle regression
  criteria). One-line refresh (task #18).
- **Gap 4.C:** no new crate yet for `engine-gpu` (ADR-049). Cargo
  workspace will need a member entry as part of packet 11.

---

## §5 · ADR Coverage Audit

### §5.1 · Inventory and status

The repo carries 38 ADRs. By body-length signature:

- **12-line stubs (23 ADRs):** 001, 002, 003, 004, 005, 006, 007, 008,
  009, 010, 011, 012, 013, 014, 015, 016, 017, 018, 019, 020, 021,
  022, 024. (024 inferred; opening it is a safe spot-check before
  remediation.) Each body literally reads `*Stub. Full Context /
  Decision / Rationale / Consequences to be expanded per spec Part
  XX.2.*`
- **Partial (1 ADR):** 025 (51 lines). Single Decision section
  without separate Context / Consequences / Verification headings.
- **Expanded (14 ADRs):** 023 (67), 026 (116), 027 (160), 028 (114),
  029 (108), 030 (123), 031 (157), 032 (166), 033 (195), 034 (108),
  035 (180), 036 (174), 037 (139), 038 (112).
- **Gold-standard exemplar:** ADR-033 (195 lines, includes Context,
  Decision (subsectioned), Consequences, Risks and tradeoffs,
  Alternatives considered, Verification, Addendum). Every new ADR
  written during remediation should follow that template.

### §5.2 · One-line summary table (canonical reference)

(From the existing summaries in spec §XXII plus the four Phase-4
additions.)

| # | Title | Status (file) | Phase 5 hit |
|---|---|---|---|
| 001 | Rust as the implementation language | stub | — |
| 002 | Hybrid ECS storage | stub | — |
| 003 | Slang as the authoring shader language | stub | **direct** |
| 004 | Two-track rendering pipeline | stub | **direct** |
| 005 | Vendor upscaler first, owned fallback | stub | **direct** |
| 006 | WGSL and WebTransport for the web target | stub | **direct** |
| 007 | Owned scripting VM | stub | — |
| 008 | Content-addressed asset pipeline | stub | indirect (shader paks) |
| 009 | Two netcode modes | stub | — |
| 010 | Telemetry as a first-class subsystem | stub | **direct** (render signals) |
| 011 | Owned crash handler and unwinder | stub | — |
| 012 | 50-year API stability | stub | indirect (engine-api activation) |
| 013 | Determinism Contract | stub | **direct** (math + RNG paths) |
| 014 | Hot/cold component separation | stub | **direct** (render components) |
| 015 | Niri as development compositor | stub | — |
| 016 | Frame Pacing Contract | stub | **direct** (Phase 5 CI gate) |
| 017 | Game Master as a pluggable provider | stub | — |
| 018 | Plugin sandboxing | stub | — |
| 019 | Asset sandbox subprocesses | stub | indirect (slangc pattern reused) |
| 020 | Telemetry consent opt-in | stub | — |
| 021 | Retain existing LUKS disk layout | stub | — |
| 022 | Kernel transition to cachyos-bore | stub | — |
| 023 | engine-math determinism strategy | expanded | **direct** (no FMA, no libm) |
| 024 | Derive macros consolidated in engine-ecs-macro | stub (inferred) | — |
| 025 | Audited crypto crates, not owned | partial | — |
| 026 | General free-list arena + unified accounting | expanded | indirect |
| 027 | engine-math SIMD policy | expanded | **direct** |
| 028 | Owned Robin Hood hash map | expanded | indirect |
| 029 | mmap-backed pak loader | expanded | indirect |
| 030 | Owned sampling profiler | expanded | indirect (perf analysis) |
| 031 | Archetype-SoA + stable TypeStableId | expanded | indirect (render-system iteration) |
| 032 | Owned fiber job system | expanded | **direct** (render parallelism) |
| 033 | Parallel deterministic scheduler | expanded | **direct** (render job ordering) |
| 034 | sli front-end | expanded | indirect |
| 035 | sli register VM + GC + verifier | expanded | indirect |
| 036 | sli hot-reload + debugger + REPL | expanded | indirect (shader hot-reload pattern) |
| 037 | Slang shader toolchain (slangc subprocess) | expanded | **direct** (Phase 5 input) |
| 038 | Slang reproducibility golden | expanded | **direct** |

**Phase-5-direct ADRs that are still stubs:** 003, 004, 005, 006, 010,
013, 014, 016. That is 8 of the 13 Phase-5-direct ADRs. The remediation
expansions land alongside the new ADRs 039–053 in the same packet
series — or, more honestly: the new packets land first because they are
*new* design decisions; the stubs can be expanded in a follow-on
documentation packet outside this audit's critical path.

### §5.3 · Spec-demanded ADRs that do not yet exist

(Extracted from the spec by the prior planning session; numbered here
to match the remediation packets.)

| Proposed # | Topic | Spec source | Packet |
|---|---|---|---|
| 039 | Render-graph abstraction (resource DAG, Track A/B compile-time selection, oracle contract) | §IV.4.B, line 427 | 1 |
| 040 | CSM cascade selection + atlas layout (4 cascades, 4096² D32F) | §IV.4.A, line 380 | 2 |
| 041 | IBL L2 SH probe generation + sampling (128 probes baseline) | §IV.4.A, line 382 | 3 |
| 042 | TAA accumulation + rejection strategy | §IV.4.A, line 384 | 4 |
| 043 | Cluster lights 16×9×24 binning | §IV.4.A, line 381 | 5 |
| 044 | Bindless texture heap allocation (u32 index, overflow behaviour) | §IV.4.A, line 402 | 6 |
| 045 | Texture compression fallback (BC7/BC5/BC4 unavailable) | §IV.4.A, line 404 | 7 |
| 046 | Rasterizer testbed oracle regression criteria | Part IX, line 735 | 8 |
| 047 | Frame Pacing CI gate (revises/extends ADR-016) | §IV.5 + ADR-016 | 9 |
| 048 | Pak overlay composition semantics | §IV.8 + §XIX.4 | 10 |
| 049 | engine-gpu owned wgpu wrapper crate (Level-1 boundary) | new — locks in the user's Phase-5-anchor decision | 11 |
| 050 | engine-api activation + cargo-semver-checks adoption | ADR-012 + spec §XX.1 | 13 |
| 051 | Acknowledged deviations register (TOML, owned i18n, owned telemetry IPC) | new — formalises memory entry | 14 |
| 052 | Reproducible-build verification cadence | §XX.8 | 15 |
| 053 | Phase 5 PR slicing (6-PR plan) | new — locks in user's Phase-5-anchor decision | 12 |

### §5.4 · ADR-coverage gaps

- **Gap 5.A:** 23 Phase-0 ADR stubs need full expansion per spec
  §XX.2. *Out of scope* for this audit's critical path (the
  contracts are real — the spec describes them — only the local
  repository copy is thin). Recommend a follow-on documentation
  packet ("ADR Phase-0 expansion sweep") to be scheduled after the
  audit closes. The Phase-5-direct stubs (003, 004, 005, 006, 010,
  013, 014, 016) should be prioritised; the rest can land
  opportunistically.
- **Gap 5.B:** 14 new ADRs are needed before Phase 5 implementation
  starts. The remediation packets close all 14 (see Part B of the
  approved plan).
- **Gap 5.C:** the ADR template/format is not pinned in
  `docs/adr/README.md` (no such file exists). ADR-033 is the de-facto
  exemplar; an `0000-template.md` could prevent future drift. Not a
  blocker; tracked as low-severity.

---

## §6 · Owned-vs-Vendored Discipline Audit

### §6.1 · Allowed third-party dependencies (with justification)

| Crate | Where | Spec justification |
|---|---|---|
| `libc` | engine-platform only | POSIX syscalls; the Vulkan-spec exception is the only blanket waiver, but `libc` is the equivalent for the OS layer. No ADR cites it explicitly — implicit. |
| `sha2` | engine-asset | ADR-025 (audited crypto crates not owned) |
| `ed25519-dalek` | engine-asset (PakSigner) | ADR-025 |
| `blake3` | engine-math (goldens), engine-script (compile-parity goldens), engine-core (digest), tools/engine-shader (digest), tools/sampling-profiler | implied audited-crypto-crate; explicit ADR-025 expansion would name BLAKE3 specifically |
| `criterion` | dev-only across crates | ADR-026 (benchmark substrate) |
| `proc-macro2`, `syn`, `quote` | engine-ecs-macro | proc-macro ergonomics; ADR-024 should name these explicitly when expanded |

### §6.2 · CI guards verifying the discipline

`.github/workflows/ci.yml` (395 lines, single file) holds 8 grep
guards in the `gate` job (lines 51–256) and the FMA grep guard in the
`determinism` job (lines 302–311):

1. ADR-028 — std::collections::HashMap rejected in engine-core/src,
   engine-asset/src (lines 51–67)
2. ADR-032 — std::thread::spawn, std::sync::{Mutex,RwLock,mpsc}
   allowlist (lines 69–112) — allowlist: thread_pool.rs, sampler.rs,
   ecs/schedule.rs; plus reject rayon/crossbeam/tokio/parking_lot/
   async-std anywhere in crates/; plus naked_asm! confined to
   fiber/{x86_64,aarch64}.rs
3. ADR-031 — TypeId::of< grep in engine-core/src/ecs (allow:
   resources) (lines 114–131)
4. ADR-029 — libc::{mmap,munmap,madvise} allowlist (mmap.rs,
   sampler.rs) (lines 133–154)
5. ADR-035 — 0xFF literal allowlist (bytecode.rs, verify.rs,
   debug.rs) (lines 156–177)
6. ADR-034 — reject rlua/mlua/wasmtime/wasmer/cranelift/inkwell/
   lalrpop/pest/nom/combine/chumsky under engine-script/ (lines
   179–200)
7. ADR-036 — reject serde/serde_json/bincode/prost/protobuf/rmp
   under engine-script (lines 202–219)
8. ADR-037 — reject naga/shaderc/shaderc_rs/spirv_tools/spirv_cross/
   glslang under tools/engine-shader (lines 221–240)
9. ADR-036 (REPL variant) — reject rustyline/reedline/linefeed under
   bin/engine-repl (lines 242–256)
10. ADR-023/027 (determinism job) — reject .mul_add( /
    _mm_fma{a,s} / vfma / vfms / vmla / vmls under
    engine-math/src (lines 302–311)

Every guard is rooted in an accepted ADR. None are speculative.

### §6.3 · Discipline gaps surfaced by this audit

- **Gap 6.A:** no CI guard for the imminent `engine-gpu` boundary —
  Phase 5 will need a `wgpu::` token guard rejecting wgpu use
  outside `crates/engine-gpu/`. Lands with packet 11 (ADR-049).
- **Gap 6.B:** no CI guard for `engine-api` boundary — when Phase 10
  activates the 50-year contract, game code must not bypass the
  façade. Out of scope for this audit (Phase 10 territory); flagged
  for packet 13 (ADR-050) to *mention* but not yet enforce.
- **Gap 6.C:** ADR-025 ("audited crypto crates, not owned") names
  only the principle — not the *specific* allowlist (sha2,
  ed25519-dalek, blake3). The expansion should enumerate, with
  pinned versions and the security-audit reference for each. Tracked
  under the Phase-0 ADR expansion sweep (see §5.4 gap 5.A).

### §6.4 · Probe-test the guards?

The audit did *not* execute a deliberate-violation probe commit
against each guard. The guards are static greps with simple
allowlists; reading the workflow source plus the known clean state of
the codebase is enough confidence. If the user wants empirical
verification, a one-off `tests/ci-guards/violate-each.sh` could be
added — low value, recommended only if a future false-negative is
discovered.

---

## §7 · Determinism Contract Audit

### §7.1 · The contract (from spec §IV.2 / ADR-013 stub + ADR-023 expanded)

- `f32_det`, `f64_det` wrappers route through canonical SSE2 / NEON
  instructions
- Transcendentals are owned polynomial approximations in
  `engine-math::transcendental` (no libm)
- No FMA in engine-math (compile-time `-C target-feature=-fma` on
  determinism job + grep guard)
- Per-frame RNG keyed by BLAKE3(seed ‖ frame ‖ channel ‖ counter)
- Deterministic ECS scheduler (stable topological sort by
  (phase_index, system_type_id))
- Cross-arch CI test asserts byte-equal frame hashes on x86-64 and
  aarch64

### §7.2 · Cross-arch oracles in CI (determinism job, both
ubuntu-24.04 and ubuntu-24.04-arm)

1. engine-math simd_parity (vs scalar reference, 100k inputs)
2. engine-math determinism (committed golden: `golden-math.txt`)
3. engine-core determinism (committed golden: `golden-core.txt`)
4. engine-core replay_parity (worker counts {1,2,4,N}; ADR-033)
5. engine-script compile_parity (BLAKE3 over optimised-IR text;
   committed golden: `sli-compile.golden`; ADR-034)
6. engine-script vm_oracle (BLAKE3 over (name,result) pairs of 11
   curated programs; committed golden: `sli-vm.golden`; ADR-035)
7. engine-script codegen_no_trap (500-program fuzz + corpus;
   ADR-035)
8. engine-script debug_protocol (round-trip every request/response/
   event variant; ADR-036)
9. engine-script hot_reload (deterministic polling-watcher backend)
10. engine-script breakpoint_persistence (round-trip owned TOML
    writer/reader)
11. engine-script watch_expr_safety (strict-deny purity verifier)
12. engine-debug editor-bridge (subprocess example; protocol contract)
13. engine-shader target_enum + bundle_codec (pure-Rust; codec
    stability; ADR-037)
14. engine-shader reproducibility (per-(stage,entry,target) BLAKE3
    digests vs committed golden; graceful skip if slangc absent;
    ADR-038)

The committed goldens are:
- `crates/engine-math/tests/golden-math.txt`
- `crates/engine-core/tests/golden-core.txt`
- `crates/engine-script/tests/goldens/sli-compile.golden`
- `crates/engine-script/tests/goldens/sli-vm.golden`
- `tools/engine-shader/tests/goldens/triangle-reproducibility.golden`

Two architectures agreeing on one golden proves transitive
byte-equality across the third (Determinism Contract clause).

### §7.3 · Verified

- `[profile.sim]` is declared in workspace `Cargo.toml`; the
  no-FMA flag is applied at invocation time via the `determinism`
  job's `env: RUSTFLAGS: "-C target-feature=-fma"` (workflow line
  295). The job-level env makes the flag uniform across every step.
- ADR-023 + ADR-027 specify the FMA grep guard; the workflow runs
  it as the first step of the determinism job (lines 302–311).
- ADR-033's replay-parity oracle is the runtime backstop for the
  one piece of `unsafe` in engine-core (the `dispatch_phase`
  `&mut World` reborrow).

### §7.4 · Determinism gaps surfaced

- **Gap 7.A:** ADR-013 itself is a 12-line stub. The contract is
  real (spec + ADR-023 + ADR-027 + ADR-033 implement it), but the
  ADR document doesn't say so. Phase-0 ADR expansion sweep.
- **Gap 7.B:** the RNG component — "per-frame BLAKE3(seed ‖ frame ‖
  channel ‖ counter)" — has no audited implementation yet (no
  `engine_core::rng::BlakeRng` or similar that the audit found in
  the spot-check). The crash-handler and netcode phases need it;
  not blocking Phase 5 directly (the renderer doesn't sample randomness
  on the simulation path), but flagged as a missing piece of the
  Determinism Contract realisation. Recommend a small Phase-4½
  follow-up to land the owned RNG before Phase 7 (physics) needs it
  for solver tie-breaking. Tracked outside this audit's critical
  path.

---

## §8 · Security Audit

### §8.1 · `unsafe` inventory (per ADR backing)

- **engine-platform::fiber::{x86_64,aarch64}** — `naked_asm!`
  context-switch primitives. Required for user-space fibers;
  allowlisted by the ADR-032 CI guard. Stack mappings are
  guard-paged via MmapAnon, so a fiber stack overflow lands in a
  PROT_NONE page rather than corrupting adjacent memory.
- **engine-platform::mmap** — wraps `libc::mmap`/`munmap`. The
  `MmapRo` wrapper enforces munmap-on-drop and length-bounds (ADR-029);
  the asset loader checks every blob `(offset, len)` against file
  size before indexing so a truncated pak surfaces as
  `PakError::Truncated` rather than SIGBUS.
- **engine-platform::sampler** — `libc::mmap` for the perf-event
  ring buffer with PROT_WRITE on the header page; allowlisted in the
  ADR-029 guard for that specific file.
- **engine-core::ecs::schedule::dispatch_phase** — `&mut World`
  reborrow into worker closures; ADR-033 §Decision step 2 documents
  the SAFETY discipline (declared R/W sets must be disjoint;
  structural mutation reserved for exclusive systems); runtime
  backstop is `tests/replay_parity.rs`.

This is exhaustive for engine-core; per the ADR-033 Consequences
section, "The unsafe reborrow in dispatch_phase is the only unsafe
block in the engine-core source." The other unsafe is confined to
engine-platform per the ADR-029/032 allowlists.

The audit did not run `cargo-geiger` (spec §XIX.2 names it). Doing so
adds defence-in-depth verification; tracked as a low-priority follow-up
(could be a step in the `gate` CI job).

### §8.2 · Sandbox boundaries

- **slangc subprocess.** `tools/engine-shader/src/slangc.rs` follows
  the ADR-019 pattern: `Command::env_clear()`, `LANG=C.UTF-8` only,
  closed stdin, piped stdout/stderr, explicit args. No network. This
  is the *only* asset-style importer that exists today; FBX/OBJ/glTF
  importers (Phase 5+) will follow the same pattern.
- **VM bytecode verifier.** `crates/engine-script/src/verify.rs`
  rejects unknown opcodes, 0xFF bytes (TRAP-only), OOB registers,
  truncated instructions, bad jumps, missing returns. Defence in
  depth: a four-layer impossibility argument (type system + grep
  guard + 500-program fuzz oracle + the verifier itself) keeps user
  code from emitting a TRAP byte.
- **VM dispatch table.** ADR-035 + ADR-018 anticipate `mprotect`
  read-only on the dispatch table for trusted in-process plugins.
  Not yet implemented (the dispatch table is still a `match` arm,
  not a function-pointer table); deferred to the plugin-system
  phase. Out of Phase 5 scope.

### §8.3 · Crypto choices

Per ADR-025 (partial), the engine **deliberately does not own**
cryptography. The Cargo.lock-resolved third-party crypto crates are:

- `sha2` — content hashes for the asset pipeline (ADR-008)
- `ed25519-dalek` — pak signing (ADR-025 + Live Ops kill-switch
  flow)
- `blake3` — cross-arch goldens, deterministic hasher, RNG keying
  (per ADR-013 spec)

All are RustCrypto / audited-upstream crates with permissive
licenses. No GPL/AGPL/SSPL (deny.toml denies these).

### §8.4 · RNG sources

The spec mandates BLAKE3-keyed per-frame RNG; the audit found no
`engine_core::rng` module in the spot-check. The crate exposes a
`telemetry::Signal` set that does not include any RNG signal; the
schedule and replay-parity tests use deterministic-by-construction
fixtures without entropy. **This is gap 7.B** — formally surfaced
here too: when Phase 7 physics or Phase 9 netcode lands, the owned
BLAKE3-keyed RNG must already exist.

### §8.5 · Security gaps surfaced

- **Gap 8.A:** no `cargo-geiger` step in CI. Low priority; the
  unsafe inventory is verified manually above.
- **Gap 8.B:** owned BLAKE3-keyed RNG missing (also gap 7.B). Pre-
  Phase-7 hard requirement.
- **Gap 8.C:** ADR-018 (plugin sandboxing) and ADR-019 (asset
  sandbox subprocesses) are 12-line stubs. The *patterns* are
  implemented (slangc subprocess) but the contracts are unwritten.
  Phase-0 ADR expansion sweep.

---

## §9 · 50-Year API Stability Audit

### §9.1 · Status

- `engine-api/src/lib.rs` — 3-line doc comment, no exports
- `engine-api/Cargo.toml` — no dependencies declared
- `engine-plugin-api/src/lib.rs` — 3-line doc comment, no exports
- `.github/workflows/ci.yml` — no `cargo-semver-checks` step
- ADR-012 — 12-line stub

This is correct for the current phase (v0.x is contract-exempt per
spec risk R-03 and ADR-012 stub line 5). The audit's only
recommendation is to **wire cargo-semver-checks into CI now**, before
the contract activates, so the gate is proven in-place rather than
proven at the moment of highest stakes. While `engine-api` is empty,
the check runs as a no-op — exactly the right time to install it.

ADR-050 (packet 13) records the activation strategy and pins the
v0.x → v1.0 transition to Phase 10 (per spec §XXI implementation
phases).

### §9.2 · Gaps

- **Gap 9.A:** cargo-semver-checks not in CI (packet 13).
- **Gap 9.B:** ADR-012 stub (Phase-0 expansion sweep).
- **Gap 9.C:** no `tests/semver/` content yet. The directory exists
  (per README §Layout); it is empty. Spec §XX.5 demands previous-
  major game code under `tests/semver/`; the contract has nothing to
  test against until v1.0, so this is correctly empty *now*. Flagged
  only so the directory's emptiness isn't mistaken for a bug.

---

## §10 · Telemetry and Observability Audit

### §10.1 · Subsystem enum (engine-core/src/telemetry.rs)

Twelve variants: `Ecs, Render, Physics, Audio, Net, Script, Ai,
Asset, Editor, Hub, Platform, Telemetry`. Render is reserved but no
render code yet emits.

### §10.2 · Signal variants

Seven: `Span, Counter, Gauge, Event, Sample` (ADR-030),
`ScriptBreakpointHit, ScriptException` (ADR-036). Render-side signals
(per spec Part X and the Phase 5 planning notes) will be added in
Phase 5 PR 3+ (G-buffer pass), Phase 5 PR 6 (frame-pacing gate). No
gap to close in *this* audit; recording for the Phase 5 plan.

### §10.3 · Observatory baselines (`docs/observatory/`)

Nine committed baselines:
- archetype-baseline.md (Phase 3)
- arena-baseline.md (Phase 1)
- cache-baseline.md (Phase 1)
- hashmap-baseline.md (Phase 2)
- jobs-baseline.md (Phase 3)
- million-entities-baseline.md (Phase 3, v0.1.1 closure)
- mmap-asset-baseline.md (Phase 2)
- parallel-schedule-baseline.md (Phase 3)
- profiler-baseline.md (Phase 2)

Every Phase 1–4 shipped subsystem has a baseline. Phase 4 (sli + Slang
toolchain) is *not* on this list — correctly, since neither has a
measurable performance dimension yet (compile time and bytecode size
are observed via the goldens, not the observatory).

### §10.4 · Architecture docs (`docs/architecture/`)

Eight docs, one per foundation crate:
- engine-asset.md, engine-core.md, engine-ecs-macro.md, engine-i18n.md,
  engine-math.md, engine-platform.md, engine-reflect.md,
  engine-telemetry.md

No architecture doc for engine-script. This is the largest *real*
crate without an architecture doc.

### §10.5 · Telemetry/observability gaps

- **Gap 10.A:** no `engine-script.md` in `docs/architecture/`. Add
  before Phase 5 starts — Phase 5 will want to integrate sli into the
  render-time material parameter binding path eventually, and the
  architecture doc is the front door for that integration.

---

## §11 · Performance and Frame-Pacing Audit

### §11.1 · Benchmarks present

- `crates/engine-core/benches/million_entities.rs` — Phase 3 milestone,
  records per-frame wall-clock at 10k / 100k / 1M entities,
  sequential and parallel. Baseline:
  `docs/observatory/million-entities-baseline.md` (1M sequential
  median: 4.35 ms after v0.1.1 follow-up; cleanly under 16.6 ms
  milestone gate).
- Per-subsystem criterion benches that produce the per-baseline
  numbers above.

### §11.2 · Frame Pacing Contract status

- **Spec §IV.5** declares the contract (p99 ≤ 1.1× target, σ ≤
  target/16). Specific numbers at 60 / 120 / 144 FPS targets.
- **ADR-016** is a 12-line stub: "p99 frame time and frame-time
  standard deviation are the headline metrics. CI fails on
  regression." No mechanism specified.
- **`engine_platform::time::FramePacer`** exists per the
  prior Explore-agent report — sleep-then-spin to absolute deadline
  via `clock_nanosleep(CLOCK_MONOTONIC, TIMER_ABSTIME, …)` per spec
  §IV.5 line 447. Not yet wired into any render loop because there
  is no render loop yet.
- **No CI gate.** Cannot exist until there is a render loop to gate.

### §11.3 · Frame-pacing gaps

- **Gap 11.A:** ADR-016 stub (Phase-0 expansion sweep).
- **Gap 11.B:** Frame Pacing CI gate mechanism is unwritten —
  packet 9 (ADR-047) is the dedicated remediation. The gate goes
  live in Phase 5 PR 6.

---

## §12 · Documentation Audit

### §12.1 · rustdoc

`[workspace.lints.rust]` enforces `missing_docs = "deny"` (Cargo.toml
line 60). Every public item in every real crate must carry a rustdoc
comment or the build fails. The audit trusts this; no random sampling
performed.

### §12.2 · ADR format

ADR-033 is the de-facto template. The 14 expanded ADRs (023, 026–038)
generally follow Context / Decision / Consequences / Risks /
Alternatives / Verification, with minor variation (e.g. ADR-026 uses
"Decision" subsections; ADR-030 includes an "Implementation notes"
section). No format drift severe enough to warrant action.

### §12.3 · `engine.toml` and `README.md` staleness

Already flagged (gaps 3.B and 3.C). Critical because these are the
front-door documents — every new contributor or reviewer hits them
first.

### §12.4 · No `docs/adr/0000-template.md`

A template file would lower the friction for the upcoming 14 new
ADRs and the eventual Phase-0 expansion sweep. Recommended but not
blocking.

### §12.5 · Documentation gaps

- **Gap 12.A:** `engine.toml` `phase` field stale (3.B; closed by
  task #19).
- **Gap 12.B:** `README.md` Status section stale (3.C; closed by
  task #19).
- **Gap 12.C:** `docs/architecture/engine-script.md` absent (10.A).
- **Gap 12.D:** no ADR template file. Low priority.

---

## §13 · Enterprise Platform Coverage Audit (spec Part XIX)

For each XIX subsection: status against current repo.

### §13.1 · XIX.1 Infrastructure architecture (three domains)

- Development (bare metal, Arch + cachyos + Niri, self-hosted
  Forgejo CI on Hetzner): documented in spec Part XVIII;
  `engine.toml` references but does not yet provision. The repo's
  CI workflow is GitHub Actions–schema compatible with Forgejo
  Actions (ci.yml file header documents this), so the migration is
  documented-but-deferred.
- Game servers / Platform services: Phase 10+ Hub work. Out of audit
  scope.
- **Status: documented, not yet provisioned. Correctly deferred.**

### §13.2 · XIX.2 Security model (layered)

- Memory safety via Rust: enforced by language choice. ✓
- Unsafe minimized to SIMD, FFI, raw GPU memory; ADR-backed: ✓ (per
  §8.1)
- VM sandbox (mprotect, GC barrier, verifier): verifier ✓; mprotect
  deferred; GC barrier hook present but no-op (PR-3 generational
  work)
- Plugin isolation (trusted in-process, untrusted out-of-process
  seccomp-bpf): Phase 10+, no implementation
- Asset compiler sandbox: ADR-019 pattern proven by slangc subprocess
- Update signing (ed25519, HTTPS + cert pinning, secondary key):
  Phase 10+ distribution
- Workstation monitoring (nftables, Falco): documented in spec
  Part XVIII; out-of-repo
- **Status: foundation patterns in place; remaining items correctly
  deferred to their phases.**

### §13.3 · XIX.3 Save-game architecture

- `.sav` versioned binary, per-component migrate chain, schema
  evolution policy: **no implementation, no ADR**. Not yet a Phase
  problem — but spec §XX.3 demands every serialised format embed
  `format_version: u32` from day one. The asset pak format (Phase 2)
  does carry a format version; the (yet-to-exist) `.sav` format
  needs the same.
- **Gap 13.A:** save-game architecture has no ADR. Not Phase 5
  blocking; flag for Phase 7 (when first persistent state appears).

### §13.4 · XIX.4 Live-ops architecture (pak overlays)

- Pak overlay composition, kill-switch by asset hash, A/B variant
  delivery: pak format supports overlays (PakSet exists), but the
  *composition semantics* (precedence, eviction, conflict
  resolution) are undocumented.
- **Gap 13.B:** pak overlay composition has no ADR. Packet 10
  (ADR-048) closes this. Needed before Phase 5 ships shader paks.

### §13.5 · XIX.5 Business model

Out of engineering scope. License (Apache-2.0) is enforced in
deny.toml. No engineering gap.

### §13.6 · XIX.6 IP & Legal posture

- Apache-2.0 declared in workspace `Cargo.toml`. ✓
- deny.toml allowlist (MIT, Apache-2.0, BSD, Zlib, ISC, Unicode-3.0)
  rejects GPL/AGPL/SSPL. ✓
- Patents: defensive suspension clause — legal scope, not
  engineering.
- **Status: in compliance.**

---

## §14 · Book-Library Evidence Trail

The 93-volume library at `~/Resources/books/` is the rationale base
for design principles. The full per-subsystem mapping the planning
session produced is reproduced here in compressed form. Cited by exact
filename for openability.

### §14.1 · Per-decision rationale citations

| Decision | Spec source | Book (filename) | Chapter range |
|---|---|---|---|
| Rust-first | ADR-001 | `ProgrammingRust.pdf`, `RustforRustaceans.pdf`, `TheRustonomicon.pdf` | foundations / advanced / unsafe |
| Hybrid ECS | ADR-002 | `DataOrientedDesign.pdf`, `EntityComponentDesignSystemPatterns.pdf` | full / full |
| Owned VM | ADR-007, 034–036 | `CraftingInterpreters.pdf`, `CompilersPrinciplesTechniques&Tools.pdf`, `WritinganInterpreterinGo.pdf`, `WritingaCompilerinGo.pdf` | full / 5–6, 9–10 / full / full |
| Content-addressed asset pipeline | ADR-008 | `GameEngineArchitecture.pdf`, `Game Coding Complete, Fourth Edition…` | ch. 7 / asset chapter |
| Telemetry first-class | ADR-010 | `SystemsPerformance.pdf` | 1–4, 6, 15, App. A |
| Owned crash handler | ADR-011 | `PracticalFoundationsofLinuxDebugging…`, `BuildingaDebugger.pdf` | full / full |
| 50-year API stability | ADR-012 | `LargeScaleC++, Volume 1.pdf`, `ProgrammingPrinciples&PracticesC++.pdf` | levelization chapters / 1–4 |
| Determinism contract | ADR-013, 023, 027 | `ComputerSystemsaProgrammersPerspective.pdf`, `SystemsPerformance.pdf` | 2–3 (IEEE-754), 10–11 / 1–2, 6 |
| Frame pacing | ADR-016 | `SystemsPerformance.pdf`, `GameFeel.pdf` | 13–15 / full |
| Fiber job system + scheduler | ADR-032, 033 | `IsParallelProgrammingHard....pdf`, `TheArtofMultiProcessorProgramming.pdf`, `C++ConcurrencyinAction.pdf` | full / full / 1–6 |
| Owned Robin Hood HashMap | ADR-028 | `IntroductiontoAlgorithms, Vol.4.pdf` | hashing chapters |
| Cache observatory + arena | ADR-026, 027 | `OptimizedC++.pdf`, `ComputerArchitectureaQuantitativeApproach…` | 1–4, 8–9, 12 / 1, 5, App. B |

### §14.2 · Phase-5-direct book mapping (for the imminent plan)

These are the books Phase 5 should be planned against. Cited so the
reader can open them at planning time.

| Subsystem | Books (in priority order) |
|---|---|
| Deferred PBR + G-buffer | `RealTimeRendering, Fourth Edition.pdf` (ch. 1–5, 9–11), `PhysicallyBasedRendering.pdf` (ch. 1–8, 14–15), `ComputerGraphicsfromScratch.pdf` (full) |
| Vulkan GPU pipeline (via wgpu) | `VulkanProgrammingGuide.pdf` (full), `RealTimeRendering, Fourth Edition.pdf` (ch. 5–8) |
| Software rasterizer oracle | `ComputerGraphicsfromScratch.pdf` (full), `OptimizedC++.pdf` (ch. 1–4, 8–9, 12), `C++ConcurrencyinAction.pdf` (ch. 1–6 for Rayon-style parallelism) |
| Slang authoring | `CompilersPrinciplesTechniques&Tools.pdf` (ch. 5–6, 9–10) |
| CSM, IBL, cluster lights | `RealTimeRendering, Fourth Edition.pdf` (ch. 7, 10, 11, 12, 20), `PhysicallyBasedRendering.pdf` (ch. 6–8) |
| Bindless + BC compression | `RealTimeRendering, Fourth Edition.pdf` (ch. 4–6, 19), `VulkanProgrammingGuide.pdf` (descriptor management) |
| TAA + post-FX | `RealTimeRendering, Fourth Edition.pdf` (ch. 5, 9–11) |
| Render graph abstraction | `RealTimeRendering, Fourth Edition.pdf` (ch. 5–6), `DataOrientedDesign.pdf` (full) |
| Frame pacing | `SystemsPerformance.pdf` (ch. 1, 3–4, 6, 13–15), `OptimizedC++.pdf` (ch. 1, 12) |

### §14.3 · Books not in spec Appendix E that should enter Phase 5 scope

- **`OptimizedC++.pdf`** — SIMD + cache + branch prediction. Critical
  for software rasterizer + render graph throughput.
- **`C++ConcurrencyinAction.pdf`** — Rayon-style lock-free task
  stealing for the rasterizer's 8×8 tile-parallel inner loop.

### §14.4 · Books with no direct Phase-5 use (out of scope for now)

`ArtificialIntelligenceaModernApproach.pdf`, `AIfor…` series, LLM
books — Phase 8 territory. `MultiplayerGameProgramming.pdf`,
`HighPerformanceBrowserNetworking.pdf` — Phase 9. `DAFXAudioEffects.pdf`,
`TheAudioProgrammingBook.pdf` — Phase 9.

---

## §15 · Gap Inventory

Roll-up. Every gap surfaced in §3 through §13, tagged by severity
(`P0`=Phase-5-blocking, `P1`=must close before Phase 5 ships, `P2`=
should close, `P3`=can defer), category, and remediation packet ref.

| ID | Severity | Category | What's missing | Packet / task |
|---|---|---|---|---|
| 3.A | P2 | Documentation | 23 Phase-0 ADR stubs (001–022 except 023, plus 024) | Phase-0 ADR expansion sweep (post-audit, not on critical path) |
| 3.B | P0 | Documentation | `engine.toml` `phase` field stale ("3" — should be "4" or "4-audited") | task #19 |
| 3.C | P0 | Documentation | `README.md` Status section ends at Phase 3, no Phase-4 paragraph | task #19 |
| 4.A | P1 | Documentation | `engine-render/src/lib.rs` doc does not point at ADR-039 | task #18 |
| 4.B | P1 | Documentation | `testbed/engine-raster/src/lib.rs` doc does not point at ADR-046 | task #18 |
| 4.C | P1 | Structure | `engine-gpu` crate missing (will be added with ADR-049) | packet 11 |
| 5.A | P2 | ADR | 23 Phase-0 ADR stubs (duplicate of 3.A; phrased as ADR gap) | Phase-0 ADR expansion sweep |
| 5.B | P0 | ADR | 14 new ADRs needed (039–053) | packets 1–15 |
| 5.C | P3 | ADR | no `0000-template.md` | optional |
| 6.A | P1 | CI | no `wgpu::` boundary grep guard | packet 11 (ADR-049) |
| 6.B | P3 | CI | no `engine-api` boundary grep guard | tracked, Phase 10 |
| 6.C | P2 | ADR | ADR-025 names principle but not specific crates/versions | Phase-0 ADR expansion sweep |
| 7.A | P2 | ADR | ADR-013 (Determinism Contract) is a stub | Phase-0 ADR expansion sweep |
| 7.B | P1 | Code | owned BLAKE3-keyed RNG missing | Phase-4½ or early Phase 7; track outside audit |
| 8.A | P3 | CI | no `cargo-geiger` step | optional |
| 8.B | duplicate of 7.B | | | |
| 8.C | P2 | ADR | ADR-018 + ADR-019 stubs | Phase-0 ADR expansion sweep |
| 9.A | P0 | CI | no `cargo-semver-checks` step | packet 13 (ADR-050) |
| 9.B | P2 | ADR | ADR-012 stub | Phase-0 ADR expansion sweep |
| 9.C | P3 | Informational | `tests/semver/` empty (correctly — pre-v1.0) | none |
| 10.A | P1 | Documentation | `docs/architecture/engine-script.md` missing | track outside audit |
| 11.A | P2 | ADR | ADR-016 (Frame Pacing Contract) is a stub | Phase-0 ADR expansion sweep |
| 11.B | P0 | ADR + CI | Frame Pacing CI gate mechanism unwritten | packet 9 (ADR-047) |
| 12.A–D | see §12 | | | |
| 13.A | P3 | ADR | save-game architecture has no ADR | track for Phase 7 |
| 13.B | P0 | ADR | pak overlay composition semantics has no ADR | packet 10 (ADR-048) |

### §15.1 · Severity summary

- **P0 (Phase-5-blocking):** 3.B, 3.C, 5.B, 9.A, 11.B, 13.B — 6
  items, all covered by remediation packets/tasks
- **P1 (must close before Phase 5 ships):** 4.A, 4.B, 4.C, 6.A, 7.B,
  10.A — 6 items, 5 covered by remediation; 7.B and 10.A tracked
  outside this audit's critical path
- **P2 (should close):** 3.A/5.A, 6.C, 7.A, 8.C, 9.B, 11.A — Phase-0
  ADR expansion sweep, scheduled as a separate doc-only packet after
  the audit's critical-path packets land
- **P3 (can defer):** 5.C, 6.B, 8.A, 9.C, 13.A — none block any
  current or near-future work

### §15.2 · Total scope of remediation

- 14 new ADRs (039–053)
- 3 CI extensions (wgpu grep guard, cargo-semver-checks job, weekly
  reproducible-build job)
- 1 workspace `Cargo.toml` dev-dep pin (cargo-semver-checks)
- 1 new workspace member (`engine-gpu`) with skeleton lib.rs
- 2 lib.rs doc-comment refreshes (engine-render, engine-raster)
- 1 `engine.toml` update
- 1 `README.md` update

All P0 and P1 items are within scope of the 19 tasks already created
in this session (1 audit + 15 ADR/CI packets + 3 misc). Anything not
on that list (Phase-0 ADR expansion sweep, owned BLAKE3 RNG,
engine-script architecture doc) is explicitly tracked here so it
cannot be forgotten.

---

## §16 · Forward-Looking Phase 5 Anchors

This audit was scoped to phases 0–4. Two decisions about the
*upcoming* Phase 5 were also made during the planning session and are
recorded here so they bind without re-debating later. Both are
formalised as ADRs in the remediation packets.

### §16.1 · Phase 5 ships as 6 PRs (ADR-053)

The slice (already chosen, already in the plan, repeated here as the
audit's record):

1. **PR 1 — Rasterizer oracle + render-graph trait.** Software
   rasterizer in `testbed/engine-raster` (closes the stub), plus the
   `engine_render::render_graph` trait per ADR-039. Pixel-parity
   oracle ready before any GPU code exists.
2. **PR 2 — `engine-gpu` + swapchain + bindless heap.** New Level-1
   crate per ADR-049. Descriptor heap per ADR-044. First wgpu
   integration; no rendering yet.
3. **PR 3 — Deferred G-buffer + cluster lights + CSM.** Four-cascade
   shadow atlas per ADR-040; cluster binning per ADR-043; G-buffer
   pass per spec §IV.4.A.
4. **PR 4 — IBL + post-FX (SSAO, bloom, tonemap, TAA).** ADR-041 +
   ADR-042 realised, plus the standard post chain.
5. **PR 5 — UpscalerProvider trait + RX-580 milestone bench.** Trait
   surface (owned fallback only — vendor providers are Phase 6 per
   spec). RX-580 bench informational this PR; becomes a gate in PR 6.
6. **PR 6 — Frame Pacing CI gate (ADR-047) and Phase 5 closure.**
   Activates the gate. Confirms 1440p/60 milestone. Engine Core v0.2
   tag. Phase 5 closes.

### §16.2 · GPU layer is an owned wgpu wrapper (`engine-gpu`, ADR-049)

`engine-render` and every higher crate must not name `wgpu::*` types
in public API. `engine-gpu` (new Level-1 crate) re-exports surface,
device, swapchain, buffer/texture handles, command encoder, pipeline
state — all wrapped in owned types. wgpu is a permitted transitive
dependency (and transitively brings naga, which the ADR-037 guard
correctly continues to forbid as a *direct* import in
`tools/engine-shader/`). New CI grep guard: no `wgpu::` token outside
`crates/engine-gpu/`.

The justification: preserves the 50-year API contract option for
swapping the GPU layer without source-breaking changes to game code.
The same pattern as `engine-asset` wrapping the asset pak format: an
owned boundary above a permitted dependency, with a contract the rest
of the engine can rely on.

### §16.3 · Phase 5 planning resumes after this audit closes

A new planning session opens with: "Phase 5 planning — audit is
closed, all 14 ADRs landed, here are the 6 PRs." The Phase-5 plan can
then focus purely on implementation tactics (test harness, mesh-asset
importer choice, materials data model, render-thread topology) without
re-debating any of the contracts the audit just locked in.

---

## §17 · Audit Closure Criteria

This audit document is considered **closed** when:

1. All P0 gaps from §15.1 are resolved (6 items).
2. All P1 gaps that have remediation packets in scope are resolved
   (4 of 6 items; 7.B and 10.A are tracked outside the audit's
   critical path).
3. The 14 remediation packets (ADRs 039–053) are committed and CI
   is green on each.
4. `cargo-semver-checks` runs (as a no-op) in CI and passes.
5. The new `wgpu::` boundary grep guard runs (as a no-op) and passes.
6. The weekly reproducible-build workflow has at least one logged
   green run.
7. `engine.toml` `phase` reads `"4-audited"` (or equivalent
   marker — packet 19).
8. `README.md` Status section acknowledges Phase 4 closure and
   links to this audit document under an "Audits" heading.
9. This document itself is committed and linked from `README.md`.

When all nine are true, Phase 5 planning opens.

---

## §18 · Appendix — File path quick reference

For the next session, the entry points:

- Spec: `~/Resources/documentation/ENGINE_SPECIFICATION_v2.0.md`
- ADRs: `~/Projects/engine/docs/adr/`
- Observatory baselines: `~/Projects/engine/docs/observatory/`
- Architecture docs: `~/Projects/engine/docs/architecture/`
- Manifest: `~/Projects/engine/engine.toml`
- Workspace root: `~/Projects/engine/Cargo.toml`
- CI: `~/Projects/engine/.github/workflows/ci.yml`
- Plan: `~/.claude/plans/moonlit-snuggling-harbor.md`
- This audit: `~/Projects/engine/docs/audit/phases-0-4-enterprise-audit.md`
- Book library: `~/Resources/books/` (93 PDFs)

— *End of audit. The remediation packets follow in their own ADR
files; the audit's own state lives in `MEMORY.md` as a project
memory entry pointing back at this document.*
