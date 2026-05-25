# ADR-001 — Rust as the implementation language

- Status: Accepted
- Date: 2026-05-18 (expanded 2026-05-24 per audit §15 Phase-0 ADR sweep)
- Phase: 0 (pre-v1.0 — contract-exempt per risk R-03)
- Companion: ADR-013 (determinism contract), ADR-024 (derive macros),
  ADR-032 (owned fiber job system), ADR-033 (parallel deterministic
  scheduler)

## Context

The engine targets a 50-year operational lifetime (spec §0.1, §XX.1).
The single most consequential decision at Phase 0 is the implementation
language: every later choice — the ECS, the renderer, the scripting
VM, the editor, the toolchain — inherits that language's safety
guarantees, ecosystem, build model, and longevity.

The candidate set considered in Phase 0:

- **C++** — the incumbent for engines of this class (UE, Unreal, id
  Tech, Source). Mature graphics tooling. Memory safety is a
  perpetual run-time hazard; the Chrome and Microsoft security teams
  both publicly attribute ~70% of historical CVEs to use-after-free,
  buffer overrun, and data race classes that C++ cannot prevent at
  compile time.
- **Zig** — promising; immature ecosystem in 2026; no proc-macro
  equivalent for the derive-heavy ECS layer (ADR-024); compiler
  ABI not yet stable.
- **Rust** — memory-safe by default, eliminates the dominant C/C++
  defect class at compile time, single toolchain (`rustup`) covers
  every target the spec names (Linux/Win/Mac native, plus wasm32
  for the web target — ADR-006), proc-macro layer mature, async
  layer mature, ecosystem mature for graphics
  (`wgpu`/`naga`/`gltf`/`image`/`ash`) and tooling (`clap`, `serde`,
  `criterion`).
- **D / Nim / Crystal** — small ecosystems; would require owning the
  entire dependency stack from the bottom up; no production game-
  engine precedent at the scale this spec targets.

## Decision

The engine is implemented in Rust. The workspace pins a single
`rust-toolchain.toml` so every contributor and every CI runner
builds with the same `rustc` version. `cargo` is the only build
system the engine targets; `Cargo.lock` is committed (the
engine ships binaries, not just libraries — spec §XX.3 / ADR-052
on reproducibility).

Rust edition: latest stable edition at Phase-0 start (2024 edition
on 2026-05-18). Edition migrations land via dedicated PRs that touch
every crate atomically.

## Rationale

Three properties of Rust justify the choice:

1. **Memory safety with no GC.** The borrow checker eliminates
   use-after-free, double-free, and data-race classes at compile
   time. The engine's hot path (the 1 M-entity scheduler — ADR-033)
   needs every cycle; a GC-pause stall on the simulation thread
   would violate the frame-pacing contract (ADR-016). Rust gives
   the safety without the pause.
2. **`unsafe` as a localized escape hatch.** The two unsafe blocks
   the engine actually needs — the scheduler's `&mut World`
   reborrow (ADR-033) and the AnyVec-typed-erasure in the
   archetype storage (ADR-031) — are confined to ~30 lines total
   and backed by oracles. C++ would diffuse equivalent hazards
   across the entire codebase.
3. **Single-toolchain cross-platform.** One `rustup` install
   targets x86-64, aarch64, wasm32. The cross-arch determinism
   oracles (ADR-013) are runnable from any developer machine; no
   per-platform toolchain divergence.

The 50-year longevity argument is not "Rust will still exist in
2076" — that's unknowable. It is "the engine's bug surface is
small enough that a future port (to Carbon, to Swift, to a
language that does not yet exist) is achievable." A C++ codebase
of this scope, after 50 years of patches, would accrete defects
faster than any team could replatform.

## Consequences

- Every crate in the workspace is Rust. `engine-ecs-macro` is the
  one proc-macro crate (ADR-024); FFI is allowed only where the
  spec names a vendor binding (DLSS/FSR/XeSS in Phase 6, ADR-005;
  Slang's `slangc` subprocess in Phase 4, ADR-037).
- Build cost is the trade: a clean `cargo build --workspace
  --release` on the reference workstation (spec §XVIII) takes ~9
  minutes today; incremental builds dominate the inner-loop cost.
- Contributors learn Rust. The reference library
  (`ProgrammingRust.pdf`, `RustforRustaceans.pdf`,
  `TheRustonomicon.pdf` — spec §0.1 reading list) is committed to
  the project's reference directory.
- The `wgpu` dependency is the one major vendored crate the
  engine consumes; ADR-049 walls it behind an `engine-gpu`
  wrapper so the rest of the codebase is wgpu-agnostic.

## Risks and tradeoffs

- **Compile times.** Mitigated by the workspace's tight feature
  flag discipline (no feature combinatorics), by `cargo check`-
  centric local workflow, and by the `sccache`-friendly build
  configuration; CI uses a build cache.
- **`unsafe` discipline.** The audit's `cargo-geiger` enumeration
  (ADR-058) is the mechanism for keeping the unsafe surface
  visible; new unsafe lands by updating the baseline in the same
  PR.
- **Ecosystem churn.** The `Cargo.lock` is committed; semver-
  checks (ADR-050) flag breaking changes in the engine-api
  facade; deny.toml gates license and known-vulnerability
  thresholds.
- **Language evolution.** Edition migrations carry real risk.
  Mitigation: edition bumps are dedicated PRs, never bundled with
  feature work, gated by the full determinism + replay-parity
  oracle suite.

## Alternatives considered

- **C++ with `-fsanitize=address,undefined,thread` in CI.** Real
  bug-finding tool, but only catches what runs; the parallel
  scheduler's race surface (ADR-033) is far easier to *prevent*
  with the borrow checker than to *detect* under sanitizers.
- **Hybrid (Rust for safety-critical, C++ for graphics).** The
  FFI surface becomes the bug surface; the graphics layer is
  also where the unsafe pressure peaks, defeating the safety
  argument.
- **Zig.** Re-evaluate at Phase 11 (mobile/console) if compile
  times become a blocker and Zig's cross-compilation story
  remains its strongest argument.

## Verification

- `cargo build --workspace --release` green on x86-64 and
  aarch64 (the two architectures the determinism oracles run on,
  spec §IV.2).
- `cargo clippy --workspace --all-targets -- -D warnings` green
  in CI.
- `cargo-geiger --workspace` enumerated in
  `docs/observatory/cargo-geiger-baseline.md` (ADR-058); the
  baseline is the visible unsafe surface.
- The 50-year longevity claim is not testable today; it is
  testable by the engine still building on a 2076 toolchain
  after the appropriate edition migrations. The shorter-term
  verification is the *absence* of memory-safety CVEs in
  ship-quality builds, which the existing testing pipeline
  (clippy + sanitizers in CI + the oracle suite) already
  measures.
