//! `f32` vector types: [`Vec2`], [`Vec3`], [`Vec4`].
//!
//! These are the rendering- and gameplay-facing vectors. All arithmetic uses
//! plain IEEE-754 operations (no fused multiply-add), so it is deterministic
//! across platforms (ADR-023). `f64` vector variants are deferred until a
//! subsystem needs them.
//!
//! # Phase 1 SIMD
//!
//! [`Vec3`] and [`Vec4`] arithmetic — `Add`, `Sub`, `Neg`, scalar/vector
//! `Mul`, `Div<f32>`, [`Vec3::dot`], [`Vec4::dot`] — now routes through the
//! private four-lane [`crate::simd::Simd4f`] wrapper. The reduction order in
//! `dot` is preserved exactly (left-to-right scalar reduction over the
//! SIMD-computed products), so the cross-architecture determinism oracle
//! and the [SIMD parity oracle](../../tests/simd_parity.rs) both stay green.
//! [`Vec2`] stays scalar — two-lane SIMD has more shuffle overhead than
//! work to do.

use core::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};

use crate::simd::Simd4f;

/// A 2-component `f32` vector.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct Vec2 {
    /// X component.
    pub x: f32,
    /// Y component.
    pub y: f32,
}

/// A 3-component `f32` vector.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct Vec3 {
    /// X component.
    pub x: f32,
    /// Y component.
    pub y: f32,
    /// Z component.
    pub z: f32,
}

/// A 4-component `f32` vector.
///
/// `repr(C, align(16))` makes the natural SIMD alignment explicit: the four
/// lanes correspond directly to an SSE `__m128` / NEON `float32x4_t` register
/// (Phase 1, ADR-027). Field order matches the C ABI for interop with future
/// render and asset code.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C, align(16))]
pub struct Vec4 {
    /// X component.
    pub x: f32,
    /// Y component.
    pub y: f32,
    /// Z component.
    pub z: f32,
    /// W component.
    pub w: f32,
}

/// Shorthand constructor for [`Vec2`].
#[inline]
pub const fn vec2(x: f32, y: f32) -> Vec2 {
    Vec2 { x, y }
}

/// Shorthand constructor for [`Vec3`].
#[inline]
pub const fn vec3(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3 { x, y, z }
}

/// Shorthand constructor for [`Vec4`].
#[inline]
pub const fn vec4(x: f32, y: f32, z: f32, w: f32) -> Vec4 {
    Vec4 { x, y, z, w }
}

impl Vec2 {
    /// All components zero.
    pub const ZERO: Self = Self { x: 0.0, y: 0.0 };
    /// All components one.
    pub const ONE: Self = Self { x: 1.0, y: 1.0 };
    /// The positive X axis.
    pub const X: Self = Self { x: 1.0, y: 0.0 };
    /// The positive Y axis.
    pub const Y: Self = Self { x: 0.0, y: 1.0 };

    /// Constructs a vector from components.
    #[inline]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    /// A vector with every component set to `v`.
    #[inline]
    pub const fn splat(v: f32) -> Self {
        Self { x: v, y: v }
    }

    /// Dot product.
    #[inline]
    pub fn dot(self, o: Self) -> f32 {
        self.x * o.x + self.y * o.y
    }

    /// Squared length (avoids the `sqrt`).
    #[inline]
    pub fn length_squared(self) -> f32 {
        self.dot(self)
    }

    /// Euclidean length.
    #[inline]
    pub fn length(self) -> f32 {
        self.length_squared().sqrt()
    }

    /// Returns the unit vector, or [`Vec2::ZERO`] if the length is zero.
    #[inline]
    pub fn normalize_or_zero(self) -> Self {
        let len = self.length();
        if len > 0.0 { self / len } else { Self::ZERO }
    }

    /// Linear interpolation toward `o` by `t`.
    #[inline]
    pub fn lerp(self, o: Self, t: f32) -> Self {
        self + (o - self) * t
    }
}

impl Vec3 {
    /// All components zero.
    pub const ZERO: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };
    /// All components one.
    pub const ONE: Self = Self {
        x: 1.0,
        y: 1.0,
        z: 1.0,
    };
    /// The positive X axis.
    pub const X: Self = Self {
        x: 1.0,
        y: 0.0,
        z: 0.0,
    };
    /// The positive Y axis.
    pub const Y: Self = Self {
        x: 0.0,
        y: 1.0,
        z: 0.0,
    };
    /// The positive Z axis.
    pub const Z: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 1.0,
    };

    /// Constructs a vector from components.
    #[inline]
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    /// A vector with every component set to `v`.
    #[inline]
    pub const fn splat(v: f32) -> Self {
        Self { x: v, y: v, z: v }
    }

    /// Dot product.
    ///
    /// Computes `x*ox + y*oy + z*oz` left-to-right. The three products are
    /// done in parallel under the SIMD wrapper but reduced in scalar order,
    /// so the output is bit-identical to the pre-SIMD implementation
    /// (Phase 1 parity oracle).
    #[inline]
    pub fn dot(self, o: Self) -> f32 {
        let p = self.to_simd().mul(o.to_simd()).to_array();
        p[0] + p[1] + p[2]
    }

    /// Cross product.
    ///
    /// Stays scalar: the shuffles needed for a SIMD cross-product cost more
    /// than the three multiplies save on a single Vec3. Component order is
    /// the right-handed convention (`x = a.y*b.z - a.z*b.y`).
    #[inline]
    pub fn cross(self, o: Self) -> Self {
        Self {
            x: self.y * o.z - self.z * o.y,
            y: self.z * o.x - self.x * o.z,
            z: self.x * o.y - self.y * o.x,
        }
    }

    /// Squared length (avoids the `sqrt`).
    #[inline]
    pub fn length_squared(self) -> f32 {
        self.dot(self)
    }

    /// Euclidean length.
    #[inline]
    pub fn length(self) -> f32 {
        self.length_squared().sqrt()
    }

    /// Returns the unit vector, or [`Vec3::ZERO`] if the length is zero.
    #[inline]
    pub fn normalize_or_zero(self) -> Self {
        let len = self.length();
        if len > 0.0 { self / len } else { Self::ZERO }
    }

    /// Linear interpolation toward `o` by `t`.
    #[inline]
    pub fn lerp(self, o: Self, t: f32) -> Self {
        self + (o - self) * t
    }

    /// Extends to a [`Vec4`] with the given `w`.
    #[inline]
    pub const fn extend(self, w: f32) -> Vec4 {
        Vec4 {
            x: self.x,
            y: self.y,
            z: self.z,
            w,
        }
    }

    /// Load into a four-lane SIMD register with lane 3 set to zero.
    #[inline(always)]
    fn to_simd(self) -> Simd4f {
        Simd4f::new(self.x, self.y, self.z, 0.0)
    }

    /// Reconstruct from a SIMD register (lane 3 is discarded).
    #[inline(always)]
    fn from_simd(s: Simd4f) -> Self {
        let [x, y, z, _] = s.to_array();
        Self { x, y, z }
    }
}

impl Vec4 {
    /// All components zero.
    pub const ZERO: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 0.0,
        w: 0.0,
    };
    /// All components one.
    pub const ONE: Self = Self {
        x: 1.0,
        y: 1.0,
        z: 1.0,
        w: 1.0,
    };

    /// Constructs a vector from components.
    #[inline]
    pub const fn new(x: f32, y: f32, z: f32, w: f32) -> Self {
        Self { x, y, z, w }
    }

    /// A vector with every component set to `v`.
    #[inline]
    pub const fn splat(v: f32) -> Self {
        Self {
            x: v,
            y: v,
            z: v,
            w: v,
        }
    }

    /// Dot product.
    ///
    /// Computes `((x*ox + y*oy) + z*oz) + w*ow` (left-to-right). The four
    /// products are done in parallel via SIMD but reduced in scalar order,
    /// so the result is bit-identical to the pre-SIMD code path.
    #[inline]
    pub fn dot(self, o: Self) -> f32 {
        let p = self.to_simd().mul(o.to_simd()).to_array();
        ((p[0] + p[1]) + p[2]) + p[3]
    }

    /// Squared length (avoids the `sqrt`).
    #[inline]
    pub fn length_squared(self) -> f32 {
        self.dot(self)
    }

    /// Euclidean length.
    #[inline]
    pub fn length(self) -> f32 {
        self.length_squared().sqrt()
    }

    /// Drops the `w` component, yielding a [`Vec3`].
    #[inline]
    pub const fn truncate(self) -> Vec3 {
        Vec3 {
            x: self.x,
            y: self.y,
            z: self.z,
        }
    }

    /// Linear interpolation toward `o` by `t`.
    #[inline]
    pub fn lerp(self, o: Self, t: f32) -> Self {
        self + (o - self) * t
    }

    /// Load into a four-lane SIMD register.
    #[inline(always)]
    pub(crate) fn to_simd(self) -> Simd4f {
        Simd4f::new(self.x, self.y, self.z, self.w)
    }

    /// Reconstruct from a SIMD register.
    #[inline(always)]
    pub(crate) fn from_simd(s: Simd4f) -> Self {
        let [x, y, z, w] = s.to_array();
        Self { x, y, z, w }
    }
}

// --- Vec2 arithmetic (scalar) ----------------------------------------

impl Add for Vec2 {
    type Output = Self;
    #[inline]
    fn add(self, o: Self) -> Self {
        Self {
            x: self.x + o.x,
            y: self.y + o.y,
        }
    }
}

impl Sub for Vec2 {
    type Output = Self;
    #[inline]
    fn sub(self, o: Self) -> Self {
        Self {
            x: self.x - o.x,
            y: self.y - o.y,
        }
    }
}

impl Neg for Vec2 {
    type Output = Self;
    #[inline]
    fn neg(self) -> Self {
        Self {
            x: -self.x,
            y: -self.y,
        }
    }
}

impl Mul<f32> for Vec2 {
    type Output = Self;
    #[inline]
    fn mul(self, s: f32) -> Self {
        Self {
            x: self.x * s,
            y: self.y * s,
        }
    }
}

impl Mul<Vec2> for f32 {
    type Output = Vec2;
    #[inline]
    fn mul(self, v: Vec2) -> Vec2 {
        Vec2 {
            x: self * v.x,
            y: self * v.y,
        }
    }
}

impl Mul for Vec2 {
    type Output = Self;
    #[inline]
    fn mul(self, o: Self) -> Self {
        Self {
            x: self.x * o.x,
            y: self.y * o.y,
        }
    }
}

impl Div<f32> for Vec2 {
    type Output = Self;
    #[inline]
    fn div(self, s: f32) -> Self {
        Self {
            x: self.x / s,
            y: self.y / s,
        }
    }
}

impl AddAssign for Vec2 {
    #[inline]
    fn add_assign(&mut self, o: Self) {
        self.x += o.x;
        self.y += o.y;
    }
}

impl SubAssign for Vec2 {
    #[inline]
    fn sub_assign(&mut self, o: Self) {
        self.x -= o.x;
        self.y -= o.y;
    }
}

// --- Vec3 arithmetic (SIMD-backed) -----------------------------------
//
// Every operation goes through [`Simd4f`] with lane 3 zero. The wrapper's
// element-wise primitives are IEEE-correctly-rounded on every backend, so
// the output bits match the pre-SIMD scalar implementation byte-for-byte
// (Phase 1 parity oracle, ADR-027).

impl Add for Vec3 {
    type Output = Self;
    #[inline]
    fn add(self, o: Self) -> Self {
        Self::from_simd(self.to_simd().add(o.to_simd()))
    }
}

impl Sub for Vec3 {
    type Output = Self;
    #[inline]
    fn sub(self, o: Self) -> Self {
        Self::from_simd(self.to_simd().sub(o.to_simd()))
    }
}

impl Neg for Vec3 {
    type Output = Self;
    #[inline]
    fn neg(self) -> Self {
        Self::from_simd(self.to_simd().neg())
    }
}

impl Mul<f32> for Vec3 {
    type Output = Self;
    #[inline]
    fn mul(self, s: f32) -> Self {
        Self::from_simd(self.to_simd().mul(Simd4f::splat(s)))
    }
}

impl Mul<Vec3> for f32 {
    type Output = Vec3;
    #[inline]
    fn mul(self, v: Vec3) -> Vec3 {
        // Match the scalar reduction order (`s * v.x`, …). With IEEE
        // correctly-rounded multiplication, lane-wise `splat(s) * v` is
        // bit-identical to `v * splat(s)`, but keep the operand order
        // mirrored so a `*` operator switch never surprises a reviewer.
        Vec3::from_simd(Simd4f::splat(self).mul(v.to_simd()))
    }
}

impl Mul for Vec3 {
    type Output = Self;
    #[inline]
    fn mul(self, o: Self) -> Self {
        Self::from_simd(self.to_simd().mul(o.to_simd()))
    }
}

impl Div<f32> for Vec3 {
    type Output = Self;
    #[inline]
    fn div(self, s: f32) -> Self {
        Self::from_simd(self.to_simd().div(Simd4f::splat(s)))
    }
}

impl AddAssign for Vec3 {
    #[inline]
    fn add_assign(&mut self, o: Self) {
        *self = *self + o;
    }
}

impl SubAssign for Vec3 {
    #[inline]
    fn sub_assign(&mut self, o: Self) {
        *self = *self - o;
    }
}

// --- Vec4 arithmetic (SIMD-backed) -----------------------------------

impl Add for Vec4 {
    type Output = Self;
    #[inline]
    fn add(self, o: Self) -> Self {
        Self::from_simd(self.to_simd().add(o.to_simd()))
    }
}

impl Sub for Vec4 {
    type Output = Self;
    #[inline]
    fn sub(self, o: Self) -> Self {
        Self::from_simd(self.to_simd().sub(o.to_simd()))
    }
}

impl Neg for Vec4 {
    type Output = Self;
    #[inline]
    fn neg(self) -> Self {
        Self::from_simd(self.to_simd().neg())
    }
}

impl Mul<f32> for Vec4 {
    type Output = Self;
    #[inline]
    fn mul(self, s: f32) -> Self {
        Self::from_simd(self.to_simd().mul(Simd4f::splat(s)))
    }
}

impl Mul<Vec4> for f32 {
    type Output = Vec4;
    #[inline]
    fn mul(self, v: Vec4) -> Vec4 {
        Vec4::from_simd(Simd4f::splat(self).mul(v.to_simd()))
    }
}

impl Mul for Vec4 {
    type Output = Self;
    #[inline]
    fn mul(self, o: Self) -> Self {
        Self::from_simd(self.to_simd().mul(o.to_simd()))
    }
}

impl Div<f32> for Vec4 {
    type Output = Self;
    #[inline]
    fn div(self, s: f32) -> Self {
        Self::from_simd(self.to_simd().div(Simd4f::splat(s)))
    }
}

impl AddAssign for Vec4 {
    #[inline]
    fn add_assign(&mut self, o: Self) {
        *self = *self + o;
    }
}

impl SubAssign for Vec4 {
    #[inline]
    fn sub_assign(&mut self, o: Self) {
        *self = *self - o;
    }
}

// Layout invariants (Phase 1 cache observatory).
//
// These const assertions are tripwires: if a future change accidentally
// re-orders fields, bumps the size, or weakens the alignment of a hot vector
// type, the build fails at this line — long before a benchmark notices the
// regression. `Vec3` stays 12 bytes / align-4 deliberately (ABI compatibility
// with glTF and render buffers; ADR-027); `Vec4` is align-16 to match an SSE
// lane / NEON register.
const _: () = assert!(core::mem::size_of::<Vec2>() == 8);
const _: () = assert!(core::mem::align_of::<Vec2>() == 4);
const _: () = assert!(core::mem::size_of::<Vec3>() == 12);
const _: () = assert!(core::mem::align_of::<Vec3>() == 4);
const _: () = assert!(core::mem::size_of::<Vec4>() == 16);
const _: () = assert!(core::mem::align_of::<Vec4>() == 16);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dot_and_cross() {
        assert_eq!(vec3(1.0, 0.0, 0.0).dot(vec3(1.0, 0.0, 0.0)), 1.0);
        assert_eq!(vec3(1.0, 0.0, 0.0).cross(vec3(0.0, 1.0, 0.0)), Vec3::Z);
    }

    #[test]
    fn arithmetic() {
        assert_eq!(vec2(1.0, 2.0) + vec2(3.0, 4.0), vec2(4.0, 6.0));
        assert_eq!(vec3(1.0, 2.0, 3.0) * 2.0, vec3(2.0, 4.0, 6.0));
        assert_eq!(2.0 * vec3(1.0, 2.0, 3.0), vec3(2.0, 4.0, 6.0));
        assert_eq!(-vec4(1.0, 2.0, 3.0, 4.0), vec4(-1.0, -2.0, -3.0, -4.0));
    }

    #[test]
    fn normalization() {
        let n = vec3(3.0, 0.0, 4.0).normalize_or_zero();
        assert!((n.length() - 1.0).abs() < 1e-6);
        assert_eq!(Vec3::ZERO.normalize_or_zero(), Vec3::ZERO);
    }

    #[test]
    fn extend_and_truncate() {
        assert_eq!(vec3(1.0, 2.0, 3.0).extend(4.0), vec4(1.0, 2.0, 3.0, 4.0));
        assert_eq!(vec4(1.0, 2.0, 3.0, 4.0).truncate(), vec3(1.0, 2.0, 3.0));
    }
}
