# ADR-027 · engine-math SIMD policy

- Status: Accepted
- Date: 2026-05-19
- Phase: 1
- Cross-refs: ADR-013 (Determinism Contract), ADR-023 (math determinism)

## Context

Phase 0 shipped `engine-math` as a scalar IEEE-754 library (ADR-023). That
left throughput on the table for code that does a lot of vector and matrix
arithmetic: per-instance transforms in the renderer, per-frame `Mat4 * Vec4`
in the visibility pipe, gameplay code multiplying chains of `Mat4`s in
`from_trs`. Phase 1's `SILICON → C` deliverable (spec XXI) asks us to
*understand the machine model* — and a SIMD `Vec3`/`Vec4` rewrite is the
clearest way to demonstrate that understanding without sacrificing the
determinism guarantee.

The Determinism Contract (spec IV.2, ADR-013) requires that every supported
architecture produce byte-identical simulation results. ADR-023 banned
`mul_add` and the system math library and verified the result with
golden-digest oracles. A SIMD rewrite must not break any of that — the
committed `golden-math.txt` and `golden-core.txt` files must continue to
pass on both x86-64 and aarch64.

Two further constraints:

- **No nightly toolchain.** `rust-toolchain.toml` pins stable 1.95, so the
  nascent `core::simd` portable API (still nightly) is out. We need to use
  `core::arch` intrinsics directly.
- **No third-party SIMD crate.** R-02 (own every layer); `glam`, `wide`, and
  `safe_arch` are off the table.

## Decision

### Stable `core::arch` intrinsics behind a private wrapper

`engine-math` introduces a private module, [`simd`](../../crates/engine-math/src/simd.rs),
exposing one type:

```rust
pub(crate) struct Simd4f(/* opaque */);

impl Simd4f {
    pub fn new(x: f32, y: f32, z: f32, w: f32) -> Self;
    pub fn splat(v: f32) -> Self;
    pub fn add(self, o: Self) -> Self;
    pub fn sub(self, o: Self) -> Self;
    pub fn mul(self, o: Self) -> Self;
    pub fn div(self, o: Self) -> Self;
    pub fn neg(self) -> Self;
    pub fn to_array(self) -> [f32; 4];
}
```

Three backends, selected at compile time:

| Target | Backend | Intrinsics |
| --- | --- | --- |
| `x86_64` | `__m128` | `_mm_add_ps`, `_mm_sub_ps`, `_mm_mul_ps`, `_mm_div_ps`, `_mm_set_ps`, `_mm_set1_ps`, `_mm_storeu_ps` |
| `aarch64` | `float32x4_t` | `vaddq_f32`, `vsubq_f32`, `vmulq_f32`, `vdivq_f32`, `vld1q_f32`, `vdupq_n_f32`, `vst1q_f32` |
| other | `[f32; 4]` | direct scalar arithmetic |

The wrapper is `pub(crate)` — `engine-math`'s public surface stays scalar so
downstream code never has to think about which architecture it's compiling
for, and so future replacement of the wrapper (e.g. once `core::simd`
stabilises) is a localised change.

### No FMA, ever

ADR-023's `mul_add` ban stays. In Phase 1 we additionally extend the CI
guard to reject the SIMD-FMA intrinsics literally, since a future drive-by
optimisation could reach for `_mm_fmadd_ps` or `vfmaq_f32` without going
through `mul_add`:

```bash
if grep -rnE '\.mul_add\(|_mm_fma|_mm_fms|vfma|vfms|vmla|vmls' crates/engine-math/src; then
  echo "::error::engine-math must not call FMA — see ADR-023 / ADR-027"
  exit 1
fi
```

This grep covers both the high-level `mul_add` and the SIMD-FMA intrinsics
across SSE/AVX-512 (`_mm_fmadd_ps`, `_mm_fmsub_ps`, …) and NEON
(`vfmaq_f32`, `vfmsq_f32`, `vmlaq_f32`, `vmlsq_f32` — the older NEON
multiply-accumulate is contract-prone too).

### Element-wise SIMD is bit-equivalent to scalar

IEEE-754 mandates correctly-rounded results for `+ - * / sqrt`. SSE2 and
NEON implement those operations lane-by-lane with the same rounding rules
as the scalar pipeline (Rust on `x86_64` lowers scalar `f32` through scalar
SSE2 already; aarch64 uses NEON/VFP). So
`(a + b)` lane-by-lane in SIMD produces the same bits as
`(a[0]+b[0], a[1]+b[1], a[2]+b[2], a[3]+b[3])` scalar.

We rely on this to keep the `engine-math` public output byte-identical to
the pre-SIMD code. The parity oracle (`tests/simd_parity.rs`) checks 100,000
randomised inputs and asserts `to_bits()` equality against a frozen scalar
reference (`src/vec_scalar_reference.rs`) on every output.

### Reductions stay in scalar order

For operations that reduce a vector to a scalar (`dot`, matrix cells), we
preserve the pre-SIMD accumulation order *exactly*:

| Operation | Order |
| --- | --- |
| `Vec3::dot` | `x*ox + y*oy + z*oz` |
| `Vec4::dot` | `((x*ox + y*oy) + z*oz) + w*ow` |
| `Vec3::cross` | scalar throughout (shuffle cost exceeds win) |
| `Mat3::mul` | per-cell, `((0 + p0) + p1) + p2` over `k` ascending |
| `Mat4::mul` | per-cell, `((0 + p0) + p1) + p2 + p3` over `k` ascending |
| `Mat4 * Vec4` | `col(0)*splat(v.x) + col(1)*splat(v.y) + col(2)*splat(v.z) + col(3)*splat(v.w)` |

For the matrix multiplies, the SIMD path accumulates **columns** in parallel
(four cells of one output column at once) but the per-cell sum stays in
ascending-k order — same bits.

### What stays scalar

- `Vec2` — two lanes only; SIMD packing cost exceeds the work.
- `Vec3::cross` — needs two shuffles; the three multiplies are not worth
  it on a single cross product.
- `Mat4::inverse` and `Mat3::inverse` — cofactor expansions with 60+
  multiplications and a complex reduction order. SIMD reordering risk
  high, throughput win low (these are not per-frame hot paths).
- `Quat::slerp`, `Quat::rotate`, `Quat::from_axis_angle` — interpolation and
  trig route through `engine-math::transcendental`, which is owned scalar
  IEEE for determinism.
- `transcendental.rs` — owned polynomial approximations; SIMD parallelism
  would change reduction order on every step.

### `Vec4` alignment

`Vec4` is `#[repr(C, align(16))]` so its memory layout matches an SSE
`__m128` / NEON `float32x4_t` register. `Vec3` keeps `#[repr(C)]` (12 bytes,
align 4) for ABI compatibility with glTF buffers and future render code —
the one shuffle to load `(x, y, z, 0)` is cheaper than a 16-byte alignment
requirement on every Vec3-sized record in memory.

A compile-time `const _: () = assert!(size_of::<T>() == N)` set in
`vec.rs`, `mat.rs`, and `quat.rs` makes layout drift a build-time failure.

## Consequences

- `engine-math` public output is bit-identical to the pre-SIMD code (Phase 1
  parity oracle, 100,000 random inputs). The committed `golden-math.txt` and
  `golden-core.txt` files are unchanged.
- The CI guard widens to reject SIMD-FMA intrinsics — anyone adding a new
  hot path that reaches for `vfmaq_f32` will hit the grep before merge.
- One new private file (`src/simd.rs`, ~200 lines), one new frozen file
  (`src/vec_scalar_reference.rs`, dead from `lib.rs`'s perspective, included
  via `#[path]` by the parity test only), and one new test
  (`tests/simd_parity.rs`).
- Throughput wins are real but not yet measured. Phase 1 deliberately did
  not add Criterion benches for vec/mat micros — the cross-arch parity is
  the contract, throughput is a future profiling exercise.
- When `core::simd` stabilises, `Simd4f` collapses to a `core::simd::f32x4`
  newtype with no other source changes; the public API is unaffected.
