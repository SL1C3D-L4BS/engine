//! Deterministic scalar wrappers and scalar utility functions.
//!
//! [`F32Det`] and [`F64Det`] are the spec's `f32_det` / `f64_det` wrappers
//! (renamed to satisfy Rust's type-naming lint). They mark a value as living
//! on the deterministic simulation path: their arithmetic uses only the
//! IEEE-754 correctly-rounded operations `+ - * /` and `sqrt`, and their
//! transcendental methods route through the owned [`crate::transcendental`]
//! module. A fused multiply-add is never reachable through this API. See
//! ADR-023.

use crate::transcendental;

macro_rules! det_scalar {
    (
        $(#[$meta:meta])*
        $name:ident, $t:ty,
        sin = $sin:path, cos = $cos:path, tan = $tan:path,
        exp = $exp:path, ln = $ln:path,
        atan2 = $atan2:path, acos = $acos:path, asin = $asin:path
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
        #[repr(transparent)]
        pub struct $name($t);

        impl $name {
            /// The additive identity.
            pub const ZERO: Self = Self(0.0);
            /// The multiplicative identity.
            pub const ONE: Self = Self(1.0);

            /// Wraps a raw primitive value, placing it on the deterministic path.
            #[inline]
            pub const fn new(v: $t) -> Self {
                Self(v)
            }

            /// Returns the wrapped primitive value.
            #[inline]
            pub const fn raw(self) -> $t {
                self.0
            }

            /// Correctly-rounded square root. IEEE-754 mandates that `sqrt` is
            /// correctly rounded, so this is deterministic across platforms.
            #[inline]
            pub fn sqrt(self) -> Self {
                Self(self.0.sqrt())
            }

            /// Absolute value (a sign-bit clear — deterministic).
            #[inline]
            pub fn abs(self) -> Self {
                Self(self.0.abs())
            }

            /// The smaller of two values.
            #[inline]
            pub fn min(self, other: Self) -> Self {
                Self(self.0.min(other.0))
            }

            /// The larger of two values.
            #[inline]
            pub fn max(self, other: Self) -> Self {
                Self(self.0.max(other.0))
            }

            /// Deterministic sine.
            #[inline]
            pub fn sin(self) -> Self {
                Self($sin(self.0))
            }

            /// Deterministic cosine.
            #[inline]
            pub fn cos(self) -> Self {
                Self($cos(self.0))
            }

            /// Deterministic tangent.
            #[inline]
            pub fn tan(self) -> Self {
                Self($tan(self.0))
            }

            /// Deterministic exponential.
            #[inline]
            pub fn exp(self) -> Self {
                Self($exp(self.0))
            }

            /// Deterministic natural logarithm.
            #[inline]
            pub fn ln(self) -> Self {
                Self($ln(self.0))
            }

            /// Deterministic two-argument arctangent (`atan2(self, x)`).
            #[inline]
            pub fn atan2(self, x: Self) -> Self {
                Self($atan2(self.0, x.0))
            }

            /// Deterministic arccosine.
            #[inline]
            pub fn acos(self) -> Self {
                Self($acos(self.0))
            }

            /// Deterministic arcsine.
            #[inline]
            pub fn asin(self) -> Self {
                Self($asin(self.0))
            }
        }

        impl core::ops::Add for $name {
            type Output = Self;
            #[inline]
            fn add(self, o: Self) -> Self {
                Self(self.0 + o.0)
            }
        }

        impl core::ops::Sub for $name {
            type Output = Self;
            #[inline]
            fn sub(self, o: Self) -> Self {
                Self(self.0 - o.0)
            }
        }

        impl core::ops::Mul for $name {
            type Output = Self;
            #[inline]
            fn mul(self, o: Self) -> Self {
                // Deliberately a separate multiply — never `mul_add` (ADR-023).
                Self(self.0 * o.0)
            }
        }

        impl core::ops::Div for $name {
            type Output = Self;
            #[inline]
            fn div(self, o: Self) -> Self {
                Self(self.0 / o.0)
            }
        }

        impl core::ops::Neg for $name {
            type Output = Self;
            #[inline]
            fn neg(self) -> Self {
                Self(-self.0)
            }
        }

        impl core::ops::AddAssign for $name {
            #[inline]
            fn add_assign(&mut self, o: Self) {
                self.0 = self.0 + o.0;
            }
        }

        impl core::ops::SubAssign for $name {
            #[inline]
            fn sub_assign(&mut self, o: Self) {
                self.0 = self.0 - o.0;
            }
        }

        impl core::ops::MulAssign for $name {
            #[inline]
            fn mul_assign(&mut self, o: Self) {
                self.0 = self.0 * o.0;
            }
        }

        impl core::ops::DivAssign for $name {
            #[inline]
            fn div_assign(&mut self, o: Self) {
                self.0 = self.0 / o.0;
            }
        }

        impl From<$t> for $name {
            #[inline]
            fn from(v: $t) -> Self {
                Self(v)
            }
        }

        impl From<$name> for $t {
            #[inline]
            fn from(v: $name) -> $t {
                v.0
            }
        }
    };
}

det_scalar! {
    /// Deterministic `f32` wrapper (the spec's `f32_det`).
    F32Det, f32,
    sin = transcendental::sin_f32, cos = transcendental::cos_f32,
    tan = transcendental::tan_f32, exp = transcendental::exp_f32,
    ln = transcendental::ln_f32, atan2 = transcendental::atan2_f32,
    acos = transcendental::acos_f32, asin = transcendental::asin_f32
}

det_scalar! {
    /// Deterministic `f64` wrapper (the spec's `f64_det`).
    F64Det, f64,
    sin = transcendental::sin_f64, cos = transcendental::cos_f64,
    tan = transcendental::tan_f64, exp = transcendental::exp_f64,
    ln = transcendental::ln_f64, atan2 = transcendental::atan2_f64,
    acos = transcendental::acos_f64, asin = transcendental::asin_f64
}

macro_rules! scalar_utils {
    ($t:ty, $lerp:ident, $clamp:ident, $smoothstep:ident, $saturate:ident, $sign:ident) => {
        #[doc = concat!("Linear interpolation `a + (b - a) * t`, computed without a fused multiply-add (`", stringify!($t), "`).")]
        #[inline]
        pub fn $lerp(a: $t, b: $t, t: $t) -> $t {
            a + (b - a) * t
        }

        #[doc = concat!("Clamps `v` to the inclusive range `[lo, hi]` (`", stringify!($t), "`).")]
        #[inline]
        pub fn $clamp(v: $t, lo: $t, hi: $t) -> $t {
            if v < lo {
                lo
            } else if v > hi {
                hi
            } else {
                v
            }
        }

        #[doc = concat!("Clamps `v` to `[0, 1]` (`", stringify!($t), "`).")]
        #[inline]
        pub fn $saturate(v: $t) -> $t {
            $clamp(v, 0.0, 1.0)
        }

        #[doc = concat!("Smooth Hermite interpolation across `[edge0, edge1]` (`", stringify!($t), "`).")]
        #[inline]
        pub fn $smoothstep(edge0: $t, edge1: $t, x: $t) -> $t {
            let t = $saturate((x - edge0) / (edge1 - edge0));
            t * t * (3.0 - 2.0 * t)
        }

        #[doc = concat!("Sign of `v`: `-1`, `0`, or `+1` (`", stringify!($t), "`).")]
        #[inline]
        pub fn $sign(v: $t) -> $t {
            if v > 0.0 {
                1.0
            } else if v < 0.0 {
                -1.0
            } else {
                0.0
            }
        }
    };
}

scalar_utils!(f32, lerp, clamp, smoothstep, saturate, sign);
scalar_utils!(
    f64,
    lerp_f64,
    clamp_f64,
    smoothstep_f64,
    saturate_f64,
    sign_f64
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn det_scalar_arithmetic() {
        let a = F32Det::new(2.0);
        let b = F32Det::new(3.0);
        assert_eq!((a + b).raw(), 5.0);
        assert_eq!((a * b).raw(), 6.0);
        assert_eq!((b - a).raw(), 1.0);
        assert_eq!((-a).raw(), -2.0);
        assert_eq!(F32Det::new(9.0).sqrt().raw(), 3.0);
    }

    #[test]
    fn lerp_clamp_smoothstep() {
        assert_eq!(lerp(0.0, 10.0, 0.5), 5.0);
        assert_eq!(clamp(15.0, 0.0, 10.0), 10.0);
        assert_eq!(clamp(-5.0, 0.0, 10.0), 0.0);
        assert_eq!(smoothstep(0.0, 1.0, 0.0), 0.0);
        assert_eq!(smoothstep(0.0, 1.0, 1.0), 1.0);
        assert_eq!(smoothstep(0.0, 1.0, 0.5), 0.5);
        assert_eq!(sign(-3.0), -1.0);
        assert_eq!(sign(0.0), 0.0);
    }

    #[test]
    fn det_scalar_transcendentals_route_to_owned_module() {
        let x = F64Det::new(0.5);
        assert_eq!(x.sin().raw(), transcendental::sin_f64(0.5));
        assert_eq!(x.exp().raw(), transcendental::exp_f64(0.5));
    }
}
