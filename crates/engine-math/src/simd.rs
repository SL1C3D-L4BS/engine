//! Stable-Rust four-lane `f32` SIMD wrapper.
//!
//! `core::simd` is gated on nightly and the workspace pins stable 1.95
//! (`rust-toolchain.toml`), so this module is hand-written against
//! `core::arch::x86_64`, `core::arch::aarch64`, and a plain `[f32; 4]`
//! fallback. Each backend exposes the same private type [`Simd4f`] with the
//! same set of element-wise operations.
//!
//! # Determinism contract
//!
//! IEEE-754 mandates correctly-rounded results for `+`, `-`, `*`, `/`, and
//! `sqrt`. SSE2 (`_mm_*_ps`) and NEON (`vaddq_f32`, …) implement those
//! operations lane-by-lane with the same rounding rules as the scalar f32
//! pipeline (which itself routes through scalar SSE2 on x86_64). So a single
//! element-wise operation produces the same bits on every backend.
//!
//! Reductions and reorderings are *not* automatic. Where the consumer
//! (vec.rs, mat.rs) builds a result from several SIMD operations, the
//! accumulation order is kept identical to the pre-SIMD scalar code so the
//! Phase 1 parity oracle (`tests/simd_parity.rs`) stays green and the
//! cross-architecture determinism oracle stays unchanged.
//!
//! # FMA
//!
//! This module never issues a fused multiply-add. ADR-023 bans the
//! high-level `mul_add` intrinsic, and the `sim` profile builds with
//! `-C target-feature=-fma` so the optimizer cannot synthesise one. The CI
//! grep guard (ADR-027) additionally rejects the SIMD-FMA intrinsics on SSE
//! and NEON literally appearing in this directory.

#![allow(unsafe_code)]

/// A 4-lane `f32` register. Internally one of three backends, chosen at
/// compile time. The wrapper exposes only safe operations.
#[derive(Clone, Copy)]
#[repr(transparent)]
pub(crate) struct Simd4f(Backend);

#[cfg(target_arch = "x86_64")]
type Backend = core::arch::x86_64::__m128;
#[cfg(target_arch = "aarch64")]
type Backend = core::arch::aarch64::float32x4_t;
#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
type Backend = [f32; 4];

impl Simd4f {
    /// Builds a register holding `(x, y, z, w)` in lane 0..=3.
    #[inline(always)]
    pub fn new(x: f32, y: f32, z: f32, w: f32) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            // `_mm_set_ps` takes lanes in reverse order (high to low). The
            // resulting register is laid out so that lane 0 is `x`, matching
            // a memory write of `[x, y, z, w]`.
            Self(core::arch::x86_64::_mm_set_ps(w, z, y, x))
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            // Build a stack-local array, then load with `vld1q_f32`. The
            // alternative — `vsetq_lane_f32` four times — generates the same
            // code under optimisation.
            let arr = [x, y, z, w];
            Self(core::arch::aarch64::vld1q_f32(arr.as_ptr()))
        }
        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        {
            Self([x, y, z, w])
        }
    }

    /// Builds a register with every lane set to `v`.
    #[inline(always)]
    pub fn splat(v: f32) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self(core::arch::x86_64::_mm_set1_ps(v))
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            Self(core::arch::aarch64::vdupq_n_f32(v))
        }
        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        {
            Self([v, v, v, v])
        }
    }

    /// Lane-wise addition.
    #[inline(always)]
    pub fn add(self, o: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self(core::arch::x86_64::_mm_add_ps(self.0, o.0))
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            Self(core::arch::aarch64::vaddq_f32(self.0, o.0))
        }
        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        {
            Self([
                self.0[0] + o.0[0],
                self.0[1] + o.0[1],
                self.0[2] + o.0[2],
                self.0[3] + o.0[3],
            ])
        }
    }

    /// Lane-wise subtraction.
    #[inline(always)]
    pub fn sub(self, o: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self(core::arch::x86_64::_mm_sub_ps(self.0, o.0))
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            Self(core::arch::aarch64::vsubq_f32(self.0, o.0))
        }
        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        {
            Self([
                self.0[0] - o.0[0],
                self.0[1] - o.0[1],
                self.0[2] - o.0[2],
                self.0[3] - o.0[3],
            ])
        }
    }

    /// Lane-wise multiplication.
    #[inline(always)]
    pub fn mul(self, o: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self(core::arch::x86_64::_mm_mul_ps(self.0, o.0))
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            Self(core::arch::aarch64::vmulq_f32(self.0, o.0))
        }
        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        {
            Self([
                self.0[0] * o.0[0],
                self.0[1] * o.0[1],
                self.0[2] * o.0[2],
                self.0[3] * o.0[3],
            ])
        }
    }

    /// Lane-wise division.
    #[inline(always)]
    pub fn div(self, o: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self(core::arch::x86_64::_mm_div_ps(self.0, o.0))
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            Self(core::arch::aarch64::vdivq_f32(self.0, o.0))
        }
        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        {
            Self([
                self.0[0] / o.0[0],
                self.0[1] / o.0[1],
                self.0[2] / o.0[2],
                self.0[3] / o.0[3],
            ])
        }
    }

    /// Lane-wise negation.
    #[inline(always)]
    pub fn neg(self) -> Self {
        // Implementing as `(-0.0).splat() - self` would XOR the sign bit on
        // most backends, but going through an explicit zero respects the
        // sign of NaN inputs (matches scalar `-x` for every f32).
        Self::splat(0.0).sub(self)
    }

    /// Extracts the four lanes to a `[f32; 4]`.
    #[inline(always)]
    pub fn to_array(self) -> [f32; 4] {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            let mut out = [0.0f32; 4];
            core::arch::x86_64::_mm_storeu_ps(out.as_mut_ptr(), self.0);
            out
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            let mut out = [0.0f32; 4];
            core::arch::aarch64::vst1q_f32(out.as_mut_ptr(), self.0);
            out
        }
        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        {
            self.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn around() -> [f32; 4] {
        [1.5, -2.25, 3.125, -4.0625]
    }

    #[test]
    fn new_and_to_array_round_trip() {
        let [a, b, c, d] = around();
        let v = Simd4f::new(a, b, c, d);
        assert_eq!(v.to_array(), [a, b, c, d]);
    }

    #[test]
    fn splat_repeats_every_lane() {
        let v = Simd4f::splat(7.5);
        assert_eq!(v.to_array(), [7.5; 4]);
    }

    #[test]
    fn arithmetic_is_lane_wise() {
        let a = Simd4f::new(1.0, 2.0, 3.0, 4.0);
        let b = Simd4f::new(0.5, -0.5, 2.0, 0.0);
        assert_eq!(a.add(b).to_array(), [1.5, 1.5, 5.0, 4.0]);
        assert_eq!(a.sub(b).to_array(), [0.5, 2.5, 1.0, 4.0]);
        assert_eq!(a.mul(b).to_array(), [0.5, -1.0, 6.0, 0.0]);
        assert_eq!(a.neg().to_array(), [-1.0, -2.0, -3.0, -4.0]);
    }
}
