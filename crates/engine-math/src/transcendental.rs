//! Owned, deterministic transcendental functions.
//!
//! These deliberately avoid the system math library: `libm` is not
//! bit-reproducible across `glibc` versions or CPU architectures. Every
//! function here is built from `+ - * /`, comparisons, and the exact integral
//! operations (`round`, `floor`, bit reinterpretation) — all of which IEEE-754
//! defines precisely — so results are byte-identical on every IEEE-754
//! platform. See ADR-023 and the Determinism Contract (spec IV.2).
//!
//! Each function is implemented once in `f64` and the `f32` variant is derived
//! by a deterministic narrowing cast.

const PI: f64 = core::f64::consts::PI;
const TWO_PI: f64 = 2.0 * PI;
const HALF_PI: f64 = core::f64::consts::FRAC_PI_2;
const LN2: f64 = core::f64::consts::LN_2;

/// Rounds to the nearest integer, ties away from zero.
///
/// `f64::round` lowers to the IEEE-754 `roundToIntegral` operation, which is
/// exact (no rounding error) and therefore identical on every platform.
#[inline]
fn round_det(x: f64) -> f64 {
    x.round()
}

/// Deterministic sine.
pub fn sin_f64(x: f64) -> f64 {
    if !x.is_finite() {
        return f64::NAN;
    }
    // Range-reduce to `[-PI, PI]`.
    let mut r = x - TWO_PI * round_det(x / TWO_PI);
    // Fold into `[-PI/2, PI/2]` using the symmetry of sine about `±PI/2`.
    if r > HALF_PI {
        r = PI - r;
    } else if r < -HALF_PI {
        r = -PI - r;
    }
    // Degree-9 Taylor series. With `|r| <= PI/2` the truncation error is below
    // `5e-6`, dominated by the dropped `r^11 / 11!` term.
    let r2 = r * r;
    r * (1.0
        + r2 * (-1.0 / 6.0 + r2 * (1.0 / 120.0 + r2 * (-1.0 / 5040.0 + r2 * (1.0 / 362880.0)))))
}

/// Deterministic cosine.
#[inline]
pub fn cos_f64(x: f64) -> f64 {
    sin_f64(x + HALF_PI)
}

/// Deterministic tangent.
#[inline]
pub fn tan_f64(x: f64) -> f64 {
    sin_f64(x) / cos_f64(x)
}

/// Deterministic exponential (`e^x`).
pub fn exp_f64(x: f64) -> f64 {
    if x.is_nan() {
        return f64::NAN;
    }
    if x > 709.0 {
        return f64::INFINITY;
    }
    if x < -700.0 {
        return 0.0;
    }
    // Reduce: `x = k*ln2 + r` with `r` in `[-ln2/2, ln2/2]`.
    let k = round_det(x / LN2);
    let r = x - k * LN2;
    // Degree-7 Taylor series for `exp(r)`.
    let er = 1.0
        + r * (1.0
            + r * (1.0 / 2.0
                + r * (1.0 / 6.0
                    + r * (1.0 / 24.0
                        + r * (1.0 / 120.0 + r * (1.0 / 720.0 + r * (1.0 / 5040.0)))))));
    // `2^k` by constructing the IEEE-754 exponent field directly. With `x`
    // clamped to `[-700, 709]`, `k` lands in `[-1010, 1023]`, so the biased
    // exponent `k + 1023` is always a valid normal value.
    let biased = (k as i64 + 1023) as u64;
    let two_k = f64::from_bits(biased << 52);
    er * two_k
}

/// Deterministic natural logarithm.
pub fn ln_f64(x: f64) -> f64 {
    if x.is_nan() || x < 0.0 {
        return f64::NAN;
    }
    if x == 0.0 {
        return f64::NEG_INFINITY;
    }
    if x.is_infinite() {
        return f64::INFINITY;
    }
    // Decompose `x = mantissa * 2^exp` with `mantissa` in `[1, 2)`.
    let mut bits = x.to_bits();
    let mut exp = ((bits >> 52) & 0x7ff) as i64;
    if exp == 0 {
        // Subnormal: scale up by `2^54`, then correct the exponent.
        let scaled = x * f64::from_bits(0x4350_0000_0000_0000);
        bits = scaled.to_bits();
        exp = ((bits >> 52) & 0x7ff) as i64 - 54;
    }
    let mantissa = f64::from_bits((bits & 0x800f_ffff_ffff_ffff) | 0x3ff0_0000_0000_0000);
    let e = (exp - 1023) as f64;
    // `ln(mantissa)` via `2 * atanh(s)` with `s = (m-1)/(m+1)` in `[0, 1/3]`,
    // where the series converges quickly.
    let s = (mantissa - 1.0) / (mantissa + 1.0);
    let s2 = s * s;
    let ln_m = 2.0
        * s
        * (1.0
            + s2 * (1.0 / 3.0
                + s2 * (1.0 / 5.0 + s2 * (1.0 / 7.0 + s2 * (1.0 / 9.0 + s2 * (1.0 / 11.0))))));
    e * LN2 + ln_m
}

/// Deterministic arctangent.
pub fn atan_f64(z: f64) -> f64 {
    if z.is_nan() {
        return f64::NAN;
    }
    let neg = z < 0.0;
    let mut a = z.abs();
    let mut complement = false;
    if a > 1.0 {
        a = 1.0 / a;
        complement = true;
    }
    // Minimax polynomial for `atan` on `[0, 1]`.
    let a2 = a * a;
    let mut r = a
        * (0.999_866_0
            + a2 * (-0.330_299_5 + a2 * (0.180_141_0 + a2 * (-0.085_133_0 + a2 * 0.020_835_1))));
    if complement {
        r = HALF_PI - r;
    }
    if neg { -r } else { r }
}

/// Deterministic two-argument arctangent.
pub fn atan2_f64(y: f64, x: f64) -> f64 {
    if x > 0.0 {
        atan_f64(y / x)
    } else if x < 0.0 {
        if y >= 0.0 {
            atan_f64(y / x) + PI
        } else {
            atan_f64(y / x) - PI
        }
    } else if y > 0.0 {
        HALF_PI
    } else if y < 0.0 {
        -HALF_PI
    } else {
        0.0
    }
}

/// Deterministic arccosine. The input is clamped to `[-1, 1]`.
pub fn acos_f64(x: f64) -> f64 {
    let x = x.clamp(-1.0, 1.0);
    atan2_f64((1.0 - x * x).sqrt(), x)
}

/// Deterministic arcsine. The input is clamped to `[-1, 1]`.
pub fn asin_f64(x: f64) -> f64 {
    let x = x.clamp(-1.0, 1.0);
    atan2_f64(x, (1.0 - x * x).sqrt())
}

/// Deterministic sine (`f32`).
#[inline]
pub fn sin_f32(x: f32) -> f32 {
    sin_f64(x as f64) as f32
}

/// Deterministic cosine (`f32`).
#[inline]
pub fn cos_f32(x: f32) -> f32 {
    cos_f64(x as f64) as f32
}

/// Deterministic tangent (`f32`).
#[inline]
pub fn tan_f32(x: f32) -> f32 {
    tan_f64(x as f64) as f32
}

/// Deterministic exponential (`f32`).
#[inline]
pub fn exp_f32(x: f32) -> f32 {
    exp_f64(x as f64) as f32
}

/// Deterministic natural logarithm (`f32`).
#[inline]
pub fn ln_f32(x: f32) -> f32 {
    ln_f64(x as f64) as f32
}

/// Deterministic arctangent (`f32`).
#[inline]
pub fn atan_f32(z: f32) -> f32 {
    atan_f64(z as f64) as f32
}

/// Deterministic two-argument arctangent (`f32`).
#[inline]
pub fn atan2_f32(y: f32, x: f32) -> f32 {
    atan2_f64(y as f64, x as f64) as f32
}

/// Deterministic arccosine (`f32`). The input is clamped to `[-1, 1]`.
#[inline]
pub fn acos_f32(x: f32) -> f32 {
    acos_f64(x as f64) as f32
}

/// Deterministic arcsine (`f32`). The input is clamped to `[-1, 1]`.
#[inline]
pub fn asin_f32(x: f32) -> f32 {
    asin_f64(x as f64) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Maximum absolute error tolerated against the standard library.
    const EPS: f64 = 1e-4;

    #[test]
    fn sin_cos_match_std() {
        let mut x = -10.0;
        while x < 10.0 {
            assert!((sin_f64(x) - x.sin()).abs() < EPS, "sin({x})");
            assert!((cos_f64(x) - x.cos()).abs() < EPS, "cos({x})");
            x += 0.25;
        }
    }

    #[test]
    fn exp_ln_match_std() {
        let mut x = 0.01;
        while x < 50.0 {
            assert!((ln_f64(x) - x.ln()).abs() < EPS, "ln({x})");
            let e = exp_f64(x.ln());
            assert!((e - x).abs() / x < EPS, "exp(ln({x}))");
            x += 0.37;
        }
    }

    #[test]
    fn atan2_quadrants() {
        assert!((atan2_f64(1.0, 1.0) - core::f64::consts::FRAC_PI_4).abs() < EPS);
        assert!((atan2_f64(1.0, -1.0) - 3.0 * core::f64::consts::FRAC_PI_4).abs() < EPS);
        assert!((atan2_f64(-1.0, -1.0) + 3.0 * core::f64::consts::FRAC_PI_4).abs() < EPS);
        assert!((atan2_f64(0.0, 0.0)).abs() < EPS);
    }

    #[test]
    fn acos_asin_match_std() {
        let mut x = -1.0;
        while x <= 1.0 {
            assert!((acos_f64(x) - x.acos()).abs() < EPS, "acos({x})");
            assert!((asin_f64(x) - x.asin()).abs() < EPS, "asin({x})");
            x += 0.1;
        }
    }

    #[test]
    fn out_of_range_inputs_are_handled() {
        assert!(ln_f64(-1.0).is_nan());
        assert_eq!(ln_f64(0.0), f64::NEG_INFINITY);
        assert_eq!(exp_f64(1000.0), f64::INFINITY);
        assert_eq!(exp_f64(-1000.0), 0.0);
    }
}
