//! Owned signed fixed-point types: [`I32F32`] and [`I16F16`].
//!
//! Fixed-point arithmetic is exact and integer-backed, so it is deterministic
//! regardless of floating-point environment — the spec offers it for gameplay
//! code that must be deterministic by construction (spec IV.2). Addition and
//! subtraction wrap on overflow; multiplication and division compute through a
//! wider integer to avoid intermediate overflow.

use core::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};

macro_rules! fixed_type {
    (
        $(#[$meta:meta])*
        $name:ident, repr = $repr:ty, wide = $wide:ty, frac = $frac:literal
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[repr(transparent)]
        pub struct $name($repr);

        impl $name {
            /// Number of fractional bits.
            pub const FRAC_BITS: u32 = $frac;
            /// The additive identity.
            pub const ZERO: Self = Self(0);
            /// The multiplicative identity.
            pub const ONE: Self = Self((1 as $repr) << $frac);

            /// Constructs from a whole integer.
            #[inline]
            pub const fn from_int(i: i32) -> Self {
                Self((i as $repr) << $frac)
            }

            /// Constructs directly from the raw backing integer.
            #[inline]
            pub const fn from_raw(raw: $repr) -> Self {
                Self(raw)
            }

            /// Returns the raw backing integer.
            #[inline]
            pub const fn to_raw(self) -> $repr {
                self.0
            }

            /// Constructs from an `f64` by scaling then truncating toward zero.
            #[inline]
            pub fn from_f64(v: f64) -> Self {
                let scale = ((1 as $wide) << $frac) as f64;
                Self((v * scale) as $repr)
            }

            /// Converts to `f64`.
            #[inline]
            pub fn to_f64(self) -> f64 {
                let scale = ((1 as $wide) << $frac) as f64;
                self.0 as f64 / scale
            }

            /// The integer part, truncated toward negative infinity.
            #[inline]
            pub const fn floor_int(self) -> i64 {
                (self.0 >> $frac) as i64
            }
        }

        impl Add for $name {
            type Output = Self;
            #[inline]
            fn add(self, o: Self) -> Self {
                Self(self.0.wrapping_add(o.0))
            }
        }

        impl Sub for $name {
            type Output = Self;
            #[inline]
            fn sub(self, o: Self) -> Self {
                Self(self.0.wrapping_sub(o.0))
            }
        }

        impl Mul for $name {
            type Output = Self;
            #[inline]
            fn mul(self, o: Self) -> Self {
                let p = (self.0 as $wide * o.0 as $wide) >> $frac;
                Self(p as $repr)
            }
        }

        impl Div for $name {
            type Output = Self;
            #[inline]
            fn div(self, o: Self) -> Self {
                let q = ((self.0 as $wide) << $frac) / (o.0 as $wide);
                Self(q as $repr)
            }
        }

        impl Neg for $name {
            type Output = Self;
            #[inline]
            fn neg(self) -> Self {
                Self(self.0.wrapping_neg())
            }
        }

        impl AddAssign for $name {
            #[inline]
            fn add_assign(&mut self, o: Self) {
                self.0 = self.0.wrapping_add(o.0);
            }
        }

        impl SubAssign for $name {
            #[inline]
            fn sub_assign(&mut self, o: Self) {
                self.0 = self.0.wrapping_sub(o.0);
            }
        }
    };
}

fixed_type! {
    /// Signed fixed-point with 32 integer bits and 32 fractional bits
    /// (the spec's `I32F32`), backed by an `i64`.
    I32F32, repr = i64, wide = i128, frac = 32
}

fixed_type! {
    /// Signed fixed-point with 16 integer bits and 16 fractional bits
    /// (the spec's `I16F16`), backed by an `i32`.
    I16F16, repr = i32, wide = i64, frac = 16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_through_f64() {
        for &v in &[0.0, 1.0, -1.0, 3.5, -2.25, 100.125] {
            assert!((I32F32::from_f64(v).to_f64() - v).abs() < 1e-6);
            assert!((I16F16::from_f64(v).to_f64() - v).abs() < 1e-3);
        }
    }

    #[test]
    fn arithmetic_is_exact() {
        let a = I32F32::from_int(3);
        let b = I32F32::from_int(4);
        assert_eq!((a + b).to_f64(), 7.0);
        assert_eq!((a * b).to_f64(), 12.0);
        assert_eq!((b - a).to_f64(), 1.0);
        assert_eq!((a / b).to_f64(), 0.75);
        assert_eq!((-a).to_f64(), -3.0);
    }

    #[test]
    fn identities() {
        assert_eq!(I32F32::ONE.to_f64(), 1.0);
        assert_eq!(I16F16::ONE.to_f64(), 1.0);
        assert_eq!((I16F16::from_int(5) * I16F16::ONE).to_f64(), 5.0);
        assert_eq!(I32F32::from_f64(2.5).floor_int(), 2);
        assert_eq!(I32F32::from_f64(-2.5).floor_int(), -3);
    }
}
