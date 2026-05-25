# ADR-013 — Determinism contract

- Status: Accepted
- Date: 2026-05-18 (expanded 2026-05-24 per audit §15 Phase-0 ADR sweep)
- Phase: 0 (pre-v1.0 — contract-exempt per risk R-03)
- Companion: ADR-023 (engine-math determinism strategy — the math
  side), ADR-027 (engine-math SIMD policy — strict on the sim
  path), ADR-031 (archetype storage — TypeStableId is the
  cross-arch ID), ADR-033 (parallel deterministic scheduler — the
  scheduler side), ADR-057 (owned BLAKE3 RNG — the RNG side),
  ADR-009 (netcode — rollback mode depends on this contract)

## Context

A 50-year engine that supports rollback netcode (ADR-009), replay
debugging, cross-machine determinism testing, and reproducible
builds (ADR-052) cannot get away with "approximately deterministic
under typical workloads." The contract has to be precise enough
that an oracle can verify it.

The non-determinism surface in a generic Rust engine spans:

- IEEE-754 floating-point: FMA emission differs across CPUs;
  `fast-math` flags emit non-portable arithmetic; transcendental
  functions (sin/cos/exp/log) differ across libms; flush-to-zero
  modes can vary.
- Threading: scheduler order across CPU count; race conditions
  even when no data race is present (e.g. accumulating into a
  shared float).
- Random number generation: thread-local seeds, OS-RNG draws.
- Hash-table iteration: `HashMap` iteration order is intentionally
  randomised in Rust's stdlib.
- `TypeId` and structural derives: `TypeId` is implementation-
  defined and not stable across builds.
- Time: `Instant::now`, wall clock.

Every one of these is a non-determinism vector if not addressed.

## Decision

The engine's determinism contract has six pillars:

### 1. Strict IEEE-754 on the simulation path

- `engine-math` uses strict IEEE-754 (no FMA, no fast-math) on
  the simulation path per ADR-027. The math crate's profile is
  selected at build time: `--cfg sim_path_strict`.
- Transcendentals (sin/cos/exp/log) use owned implementations
  (ADR-023) when bit-equality across libms is required; the libm
  fallback is for non-determinism-critical paths only.
- Flush-to-zero is explicitly disabled at workspace level via
  the `crt-static`-adjacent build settings.

### 2. Deterministic scheduler

- ADR-033 specifies R/W-declared system registration and
  per-phase JobGraph dispatch. The replay-parity oracle verifies
  that the same workload produces byte-identical frame digests
  at worker counts {1, 2, 4, available_parallelism()}.

### 3. BLAKE3-keyed RNG

- `engine_core::rng::Rng` (existing code at
  `crates/engine-core/src/rng.rs`; ADR-057 documents it
  retroactively) is keyed by `(seed, frame, channel, counter)`
  and is stateless-by-construction. Same seed + same frame =
  same draw, independent of all other state.

### 4. Cross-architecture byte-equal frame-hash CI test

- A canonical workload (the one in
  `crates/engine-core/tests/determinism.rs`) runs in CI on x86-64
  and aarch64. The frame digests are byte-identical. A
  divergence is a build-blocking failure.

### 5. TypeStableId instead of TypeId

- `engine_core::ecs::TypeStableId` (ADR-031) is a FNV-1a-derived
  stable type identifier; `TypeId` is unused outside ad-hoc
  test code. The component column lookup uses `TypeStableId`
  so the archetype layout is stable across builds.

### 6. No hash-table iteration order in observable state

- The engine's `HashMap`-equivalent (ADR-028 owned Robin Hood
  hash map) iterates in a deterministic insertion+hash order.
  The owned implementation guarantees a stable observable
  iteration sequence per fixed insertion sequence.

The combined invariant: **a given engine binary, given the same
inputs (asset pak hashes, script bytecode hashes, initial seed),
produces byte-identical frame digests on every architecture the
engine supports, regardless of worker count.**

## Rationale

Determinism is foundational because it makes other contracts
testable:

- Rollback netcode (ADR-009) *requires* determinism. Without it,
  every machine's simulation diverges; reconciliation is
  impossible because no two machines agree on the truth.
- Replay debugging requires determinism. A bug reproduced from a
  recorded input stream is only reproducible if the simulation
  is deterministic from those inputs.
- Reproducible builds (ADR-052) require deterministic asset
  building; that's a static-time corollary of the engine's
  runtime determinism stance.
- The audit's oracle pattern requires determinism. An oracle's
  "did this run produce the expected hash?" is only meaningful
  if the run is deterministic.

The six pillars cover the engine's known non-determinism
surface; each pillar has an owning ADR. The contract is the
*conjunction* of the pillars; weakening any one breaks the whole.

## Consequences

- Every system on the simulation path must respect strict-
  IEEE-754. Code that uses FMA (e.g. `mul_add`) is forbidden
  on the sim path; a clippy lint (workspace-wide) catches
  obvious cases; the determinism oracle catches the rest.
- Hash maps in the engine use the owned Robin Hood map
  (ADR-028), not `std::collections::HashMap`, in any path
  whose iteration order is observable.
- `TypeId` is forbidden outside test code; `TypeStableId` is
  the canonical type identifier.
- The CI cost is real: the determinism job runs on both
  x86-64 and aarch64 (the latter via QEMU on Linux runners
  until self-hosted aarch64 hardware is provisioned).
- The 9 existing cross-arch oracles (engine-math, engine-core,
  engine-script ×7, engine-shader) constitute the realisation;
  the determinism job is their union.

## Risks and tradeoffs

- **Strict IEEE-754 leaves ~10% performance on the table** on
  the sim path versus FMA-enabled compilation. Acceptable cost
  for the determinism property.
- **Owned transcendentals are slower than vendor libms.**
  Mitigation: only the sim path uses them; the renderer/audio
  use libm freely because they are not on the determinism
  contract.
- **cross-arch CI cost.** QEMU on aarch64 is slow; mitigated by
  running the determinism job nightly (informational) and on
  PRs touching the sim path (required). Self-hosted aarch64 is
  a Phase 10+ stretch.
- **Edge cases in determinism slip silently** if no test
  exercises them. Mitigation: the oracle corpus grows with
  every PR that touches the sim path; new systems land with
  new oracle entries.

## Alternatives considered

- **Approximate determinism** ("identical except for floating-
  point rounding"). Insufficient for rollback netcode and
  oracle verification. Rejected.
- **Determinism opt-in per game.** Tempting (the rollback
  user opts in; the shooter opts out). Rejected because the
  oracle property is engine-wide; making it opt-in means
  most code is unverified.
- **Use a vendor determinism crate** (e.g. `rapier`'s
  determinism mode). Useful precedent; the engine owns the
  contract for the same R-02 reasons as ADR-031 / ADR-033.
- **Determinism only on x86-64.** Loses aarch64 portability
  testing (and mobile/console, Phase 11+). Rejected.

## Verification

- **`cargo test -p engine-core --test determinism`** — the
  canonical determinism oracle. Frame digests at frames 1,
  10, 100, 1000 must match the committed golden file.
- **CI determinism job** runs the oracle on x86-64 and aarch64.
  The golden file is committed; a divergence is a build-blocking
  failure.
- **Replay-parity oracle** (`tests/replay_parity.rs`, ADR-033)
  — verifies determinism across worker counts.
- **Cross-system oracles:**
  - engine-math: `tests/determinism.rs` (canonical math
    operations, ULP-accurate).
  - engine-core: `tests/determinism.rs` + `tests/replay_parity.rs`.
  - engine-script: 7 oracle files covering parse, typeck, IR,
    bytecode, verifier, VM, GC.
  - engine-shader: slangc reproducibility golden (ADR-038).
- **The 9 oracles together form the determinism realisation.**
  A new system that introduces non-determinism would be caught
  on the first PR that triggered its execution under the
  oracle's workload.
