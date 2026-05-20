# ADR-023 · engine-math determinism strategy

- Status: Accepted
- Date: 2026-05-19
- Phase: 0 (pre-v1.0 — contract-exempt per risk R-03)

## Context

The Determinism Contract (spec IV.2, ADR-013) requires the simulation to
produce byte-identical results on every supported architecture. `engine-math`
is the foundation of that contract: every vector, matrix, quaternion, and
transcendental operation in the simulation flows through it. If the math layer
is not bit-reproducible, nothing above it can be.

Two hazards make floating-point math non-deterministic across platforms:

1. **The system math library.** `libm`'s `sin`, `cos`, `exp`, … are *not*
   correctly rounded and their results vary across `glibc` versions and CPU
   architectures.
2. **Fused multiply-add.** `a * b + c` may be contracted into a single FMA
   instruction that rounds once instead of twice, producing a different result
   from the un-fused form — and whether it is contracted depends on the target
   and the optimizer.

IEEE-754, by contrast, *mandates* correct rounding for `+`, `-`, `*`, `/`, and
`sqrt`: those five operations are bit-identical on every conforming platform.

## Decision

`engine-math` is built only from the correctly-rounded IEEE-754 operations:

- **Transcendentals are owned.** `sin`, `cos`, `tan`, `exp`, `ln`, `atan2`,
  … are owned polynomial approximations in `transcendental.rs`, built only from
  `+ - * /` and exact integral operations. `libm` is never called.
- **`sqrt` uses the hardware intrinsic.** IEEE-754 mandates correctly-rounded
  `sqrt`, so the intrinsic is deterministic — no owned approximation needed.
- **`mul_add` is never called.** `engine-math` source contains no `.mul_add(`.
  This is enforced in CI by a `grep` guard that fails the build if found.
- **The `sim` profile disables FMA codegen.** `RUSTFLAGS="-C
  target-feature=-fma"` (the `profile.sim` flag) stops the optimizer from
  contracting `a * b + c` even in code outside `engine-math` — belt and
  suspenders behind the no-`mul_add` rule.

Determinism is verified by golden-digest oracles (`tests/determinism.rs` in
`engine-math` and `engine-core`): a fixed battery of operations is reduced to
an FNV-1a digest and compared to a committed golden. CI runs the oracles on
x86-64 and aarch64 against the *same* golden, so two passing runs are
byte-identical to each other.

## Rationale

Owning the transcendentals is the one place the "own every layer" principle
(R-02) is unambiguously correct: a polynomial approximation is small, testable,
and — critically — *ours*, so its rounding behaviour cannot drift when a distro
updates `glibc`. The cost (a few hundred lines of well-understood numerics) is
low and paid once.

## Consequences

- `engine-math` transcendentals trade a small amount of accuracy for total
  reproducibility. This is the correct trade for a simulation; rendering code
  that wants peak accuracy and does not feed the simulation may use other paths.
- Any new math routine must be reviewed for `mul_add` and `libm` use. The CI
  guard catches `mul_add`; `libm` is caught by `engine-math` having no such
  dependency.
- The golden files are part of the contract. Regenerating them
  (`just gen-golden`) is an intentional, reviewed act.
