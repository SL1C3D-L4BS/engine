# ADR-057 — Owned BLAKE3 RNG (retroactive)

- Status: Accepted (retroactive — code shipped pre-ADR; ADR
  backfills the contract)
- Date: 2026-05-24 (backfilled per audit §15 / §7.B / §8.B)
- Phase: 0 (foundation; implementation already present in
  `crates/engine-core/src/rng.rs` since Phase-0 foundation
  build)
- Companion: ADR-013 (determinism contract — the property
  this RNG realises), ADR-025 (audited crypto crates — BLAKE3
  is one of them), spec §IV.2

## Context

The engine's determinism contract (ADR-013) requires a
deterministic RNG: given the same seed and the same logical
"frame," every draw must return the same value, independent
of all other engine state, on every architecture.

The standard library's RNG (`rand::thread_rng` in any
incarnation) does not satisfy this — its seeding is
non-deterministic, thread-local state leaks across phases,
and its `f32` distribution is implementation-defined.

The audit's §7.B / §8.B noted a "gap": the engine has an
RNG implementation but no ADR documenting its contract.
The implementation is present at
`crates/engine-core/src/rng.rs` (125 lines, with 5 unit
tests). This ADR backfills the contract.

## Decision

The engine ships an owned BLAKE3-keyed RNG at
`crates/engine-core/src/rng.rs`. The contract:

### Keying scheme

Every draw is `BLAKE3(seed ‖ frame ‖ channel ‖ 0xFF ‖ counter)`:

- `seed: u64` — the world's deterministic seed, set at session
  start and immutable thereafter.
- `frame: u64` — the simulation tick. The same draw at the
  same `(seed, frame)` returns the same value.
- `channel: &str` — a per-subsystem identifier
  (`"physics"`, `"ai.pathing"`, `"vfx.spawn"`, …) that
  ensures independent subsystems do not interfere.
- `0xFF` byte — a delimiter so `channel` and `counter` bytes
  cannot be confused for each other across channel-name
  lengths.
- `counter: u64` — advances per draw within
  `(seed, frame, channel)`.

The output is the first 8 bytes of the BLAKE3 finalization,
interpreted little-endian as a `u64`. Derived distributions
(`f32` in [0, 1), `bool`, range integer) are deterministic
transformations of the `u64` primitive.

### API surface

```rust
pub fn derive_u64(seed: u64, frame: u64, channel: &str, counter: u64) -> u64;

pub struct Rng { /* seed, frame, counter */ }
impl Rng {
    pub fn new(seed: u64, frame: u64) -> Self;
    pub fn next_u64(&mut self, channel: &str) -> u64;
    pub fn next_f32(&mut self, channel: &str) -> f32;
    pub fn next_bool(&mut self, channel: &str) -> bool;
    pub fn next_range(&mut self, channel: &str, low: i64, high: i64) -> i64;
}
```

### Stateless-by-construction property

`derive_u64` is pure — same inputs, same output, no hidden
state. The `Rng` struct is a convenience wrapper that
maintains a counter; `Rng::new(seed, frame)` creates a fresh
counter, so the per-frame draw is independent of any other
frame's draws.

### Per-channel isolation guarantee

Two subsystems using different channel names produce
*independent* draw sequences. The channel string is part of
the BLAKE3 input; even if subsystem A draws 1000 values on
"physics" and subsystem B draws 1000 values on "ai.pathing,"
the draw sequences do not interact.

### Cross-architecture determinism

BLAKE3 is byte-deterministic; the engine's RNG is therefore
byte-deterministic across architectures. The same
`(seed, frame, channel, counter)` produces the same `u64` on
x86-64 and aarch64.

`next_f32` is a deterministic transformation of `next_u64`
(top 24 bits / 2^24); same input, same output on all
architectures.

## Rationale

Three properties motivate the BLAKE3-keyed design:

1. **Statelessness by construction.** A buggy subsystem
   cannot corrupt another subsystem's RNG sequence because
   there is no shared state. The audit's "what if a
   pathing system draws an extra value?" question has the
   clean answer: "no effect on physics, because they're on
   different channels."
2. **Cross-architecture portability.** BLAKE3 is integer-
   arithmetic; no floating point, no architecture-specific
   crypto extensions required (BLAKE3's portable backend is
   used; SIMD acceleration is correct but not required for
   determinism).
3. **Cryptographic quality.** BLAKE3's output is
   cryptographically random; statistical-quality tests (e.g.
   PractRand) trivially pass.

The choice of BLAKE3 (vs SHA-256, which the asset pipeline
uses per ADR-008) is performance: BLAKE3 is ~6× faster than
SHA-256 in pure-Rust portable mode. Per-draw cost is
already negligible (sub-µs), but the gap matters at the
1 M-entity-per-frame portfolio.

The owned wrapper (the `Rng` struct + the convenience
functions) is the engine's contract; the primitive
(`blake3::Hasher`) is the audited dependency per ADR-025.

## Consequences

- Every engine subsystem that needs randomness uses
  `engine_core::rng::Rng` (or `derive_u64` directly).
  `rand::thread_rng` is forbidden anywhere on the
  determinism path; a workspace clippy lint (Phase 10+)
  catches violations.
- The RNG has no global state — every consumer creates its
  own `Rng::new(seed, frame)` once per frame; the channel
  parameter discriminates per-subsystem draws.
- The `channel: &str` parameter is a hot-path cost (BLAKE3
  takes the bytes); subsystems should pass short literal
  strings (`"physics"`, not a constructed string).
- The scripting VM (engine-script, ADR-007) exposes the RNG
  through its FFI; script code participates in the same
  determinism contract.
- The frame snapshot for rollback netcode (ADR-009) does not
  need to capture RNG state because the state is reconstructible
  from `(seed, frame)`.

## Risks and tradeoffs

- **`channel: &str` cost.** Each draw hashes the channel
  string. Mitigation: channel names are short literals.
- **No "global RNG" convenience.** A subsystem must
  thread `(seed, frame)` to its draw site. Mitigation:
  the scheduler exposes `(seed, frame)` on its system
  context; convenience is built on top of the primitive.
- **`next_range` modulo bias.** For non-power-of-2 ranges,
  `next_u64 % span` has slight modulo bias. Acceptable for
  game logic; cryptographic uses would require rejection
  sampling (not used in the engine).
- **The BLAKE3 dep is the dependency.** Mitigated by
  ADR-025's audit posture.

## Alternatives considered

- **`rand` crate with explicit seeding.** Works for u64
  generation; `f32` distribution is implementation-defined
  in some versions of `rand`; thread-local state is hard to
  audit. Rejected for the determinism contract.
- **xoroshiro / wyhash / splitmix.** Lower per-draw cost.
  Considered; the multi-channel independence property is
  harder to argue without keyed hashing — splitmix
  state interleaving on the same seed is the canonical
  failure mode. BLAKE3's keyed-hash approach sidesteps the
  question by reducing every draw to a content-addressed
  computation. Rejected the alternatives for the
  audit-clarity argument.
- **Per-channel `ChaCha20` instances.** Pre-2020 game-engine
  standard. Heavier setup cost per channel; weaker
  independence property when multiple channels share a seed.
  Rejected.
- **Owned BLAKE3 implementation.** Considered and rejected
  per ADR-025 — cryptographic primitives are the one
  layer the engine deliberately doesn't own.

## Verification

The existing unit tests at
`crates/engine-core/src/rng.rs` lines 75–124:

- `same_key_yields_same_value` — `derive_u64` is pure
  (same inputs, same output).
- `channels_and_counters_are_independent` — different
  channels and different counters produce different
  outputs (the basic isolation property).
- `rng_sequence_is_reproducible` — `Rng::new(seed, frame)`
  with the same arguments produces the same sequence of
  100 draws on the same channel.
- `next_f32_stays_in_unit_interval` — 1000 random `f32`
  draws are all in `[0, 1)`.
- `next_range_stays_in_bounds` — 1000 random range draws
  are all in `[-10, 10)`.

Cross-architecture verification: the engine's CI determinism
job (ADR-013) runs the engine on x86-64 and aarch64. Any
subsystem that uses the RNG indirectly contributes to the
cross-arch frame-digest determinism gate. A direct
`derive_u64` cross-arch test could be added as part of the
Phase-0 catchup PR; tracked as a stretch.

`grep -r "rand::thread_rng" crates/` — should return zero
results. The workspace's tests / sim code exclusively use
the owned RNG.
