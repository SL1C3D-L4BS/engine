# engine-math

Deterministic mathematics — the foundation of the Determinism Contract
(spec IV.2, IV.1 Level 0).

## Purpose

Provides the vector, matrix, quaternion, fixed-point, and transcendental types
the whole engine computes with. Every operation is bit-reproducible across
IEEE-754 platforms; see ADR-023 for the strategy.

## Modules

| Module             | Contents |
|--------------------|----------|
| `scalar`           | `F32Det` / `F64Det` deterministic wrappers; `lerp`, `clamp`, `smoothstep`, `sign`, `saturate`. |
| `vec`              | `Vec2`, `Vec3`, `Vec4` — arithmetic, `dot`, `cross`, `length`, `normalize`. |
| `mat`              | `Mat3`, `Mat4` — multiply, transpose, inverse, TRS / perspective / ortho constructors (column-major). |
| `quat`             | `Quat` — multiply, `slerp`, axis-angle and Euler conversion, vector rotation. |
| `fixed`            | `I32F32`, `I16F16` — integer-backed fixed-point, exact and deterministic by construction. |
| `transcendental`   | Owned polynomial `sin`/`cos`/`tan`/`exp`/`ln`/`atan2`; `sqrt` via the IEEE intrinsic. |

## Determinism

- No `mul_add`, anywhere — CI greps `engine-math/src` and fails on `.mul_add(`.
- No `libm` — transcendentals are owned approximations.
- Only IEEE-754 correctly-rounded operations (`+ - * / sqrt`) reach results.
- The `sim` profile compiles with `-C target-feature=-fma` as a second guard.

## Oracle

`tests/determinism.rs` runs a fixed battery of vec/mat/quat/fixed/transcendental
operations, reduces all result bits to an FNV-1a digest, and asserts it against
`tests/golden-math.txt`. CI runs this on x86-64 and aarch64 against the same
golden. Property tests in the unit suite check invariants such as
`normalize(v).length() ≈ 1` and `inverse(m) * m ≈ I`.

## Dependencies

`std` only — Level 0.
