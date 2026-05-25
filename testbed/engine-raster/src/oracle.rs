//! Pixel-parity oracle (ADR-046).
//!
//! Compares two `Framebuffer`s under sRGB-aware linear-space
//! differencing. The verdict is:
//!
//! - **Pass** if every pixel differs by less than `1/255` in linear
//!   space (i.e. one ULP at 8-bit sRGB output).
//! - **PassUnderThreshold** if up to `1%` of pixels exceed the
//!   per-pixel threshold but the worst delta is below
//!   `4/255`. This matches the ADR-046 p99 ≤ 1% violation
//!   relaxation.
//! - **Fail** otherwise, with the count of violating pixels and the
//!   maximum delta.
//!
//! The oracle does **not** decide whether a Fail is expected — that
//! is the exception register's job (ADR-046). The harness calls
//! `compare_images` and routes the verdict + the exception register
//! to a per-scene pass/fail decision.

use crate::framebuffer::{Framebuffer, srgb_byte_to_linear};

/// Summary of an image comparison.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageComparison {
    /// Total pixels compared.
    pub total_pixels: u64,
    /// Pixels whose max-channel linear delta exceeded `1/255`.
    pub violating_pixels: u64,
    /// Maximum linear-space delta seen across all channels.
    pub max_delta: f32,
    /// Mean linear-space delta across all pixels.
    pub mean_delta: f32,
    /// Final verdict.
    pub verdict: OracleVerdict,
}

/// Oracle verdict.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OracleVerdict {
    /// Bit-perfect within `1/255` per channel everywhere.
    Pass,
    /// Within the p99 ≤ 1% relaxation; informational.
    PassUnderThreshold,
    /// Exceeds the ADR-046 relaxation; the harness consults the
    /// exception register before declaring CI failure.
    Fail,
}

/// Compare two framebuffers per the ADR-046 contract. Both must have
/// the same extent; panics otherwise (the harness's responsibility
/// to render at matching resolutions).
pub fn compare_images(reference: &Framebuffer, candidate: &Framebuffer) -> ImageComparison {
    assert_eq!(
        (reference.width(), reference.height()),
        (candidate.width(), candidate.height()),
        "image extents differ; ADR-046 requires matching resolutions"
    );
    let total = reference.color().len() as u64;
    let mut violating = 0u64;
    let mut max_delta = 0.0f32;
    let mut sum_delta = 0.0f32;
    for (r, c) in reference.color().iter().zip(candidate.color().iter()) {
        // Decode both to linear; compare per channel.
        let rlin = [
            srgb_byte_to_linear(r.r),
            srgb_byte_to_linear(r.g),
            srgb_byte_to_linear(r.b),
        ];
        let clin = [
            srgb_byte_to_linear(c.r),
            srgb_byte_to_linear(c.g),
            srgb_byte_to_linear(c.b),
        ];
        let d = [
            (rlin[0] - clin[0]).abs(),
            (rlin[1] - clin[1]).abs(),
            (rlin[2] - clin[2]).abs(),
        ];
        let pixel_max = d[0].max(d[1]).max(d[2]);
        max_delta = max_delta.max(pixel_max);
        sum_delta += pixel_max;
        // Threshold = 1/255 in linear space.
        if pixel_max > 1.0 / 255.0 {
            violating += 1;
        }
    }
    let mean = if total > 0 {
        sum_delta / total as f32
    } else {
        0.0
    };
    let frac_violating = (violating as f32) / (total.max(1) as f32);
    let verdict = if violating == 0 {
        OracleVerdict::Pass
    } else if frac_violating <= 0.01 && max_delta < 4.0 / 255.0 {
        OracleVerdict::PassUnderThreshold
    } else {
        OracleVerdict::Fail
    };
    ImageComparison {
        total_pixels: total,
        violating_pixels: violating,
        max_delta,
        mean_delta: mean,
        verdict,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framebuffer::Rgba8;

    #[test]
    fn identical_images_pass_bit_perfect() {
        let fb1 = Framebuffer::new(8, 8);
        let fb2 = Framebuffer::new(8, 8);
        let cmp = compare_images(&fb1, &fb2);
        assert_eq!(cmp.verdict, OracleVerdict::Pass);
        assert_eq!(cmp.violating_pixels, 0);
        assert_eq!(cmp.max_delta, 0.0);
    }

    #[test]
    fn one_pixel_off_by_one_passes_under_threshold() {
        let fb1 = Framebuffer::new(10, 10);
        let mut fb2 = Framebuffer::new(10, 10);
        // One pixel: linear delta 1/255 → does not exceed the strict
        // threshold (we use strict >), so this should pass.
        fb2.write(
            0,
            0,
            Rgba8 {
                r: 1,
                g: 0,
                b: 0,
                a: 255,
            },
        );
        let cmp = compare_images(&fb1, &fb2);
        // 1/255 ≈ 0.00392; threshold check is strict > 1/255, so this
        // is right at the boundary and passes.
        assert_eq!(cmp.verdict, OracleVerdict::Pass);
    }

    #[test]
    fn large_color_delta_fails() {
        let fb1 = Framebuffer::new(8, 8);
        let mut fb2 = Framebuffer::new(8, 8);
        // Paint every pixel bright red.
        for y in 0..8 {
            for x in 0..8 {
                fb2.write(
                    x,
                    y,
                    Rgba8 {
                        r: 255,
                        g: 0,
                        b: 0,
                        a: 255,
                    },
                );
            }
        }
        let cmp = compare_images(&fb1, &fb2);
        assert_eq!(cmp.verdict, OracleVerdict::Fail);
        assert!(cmp.max_delta > 0.5);
    }

    #[test]
    fn small_fraction_off_passes_under_threshold() {
        let fb1 = Framebuffer::new(100, 100); // 10_000 pixels
        let mut fb2 = Framebuffer::new(100, 100);
        // Set 50 pixels to r=16 — linear delta ≈ 0.00485, just above
        // the 1/255 ≈ 0.00392 threshold but well below 4/255 = 0.0157.
        // 50 / 10_000 = 0.5%, well under the 1% allowance.
        for i in 0..50 {
            fb2.write(
                i as u32,
                0,
                Rgba8 {
                    r: 16,
                    g: 0,
                    b: 0,
                    a: 255,
                },
            );
        }
        let cmp = compare_images(&fb1, &fb2);
        assert_eq!(cmp.verdict, OracleVerdict::PassUnderThreshold);
        assert_eq!(cmp.violating_pixels, 50);
    }

    #[test]
    fn over_one_percent_violations_fails() {
        let fb1 = Framebuffer::new(100, 100);
        let mut fb2 = Framebuffer::new(100, 100);
        // 200 pixels = 2% — over the 1% allowance, even at the
        // small per-pixel delta.
        for i in 0..200 {
            let x = (i % 100) as u32;
            let y = (i / 100) as u32;
            fb2.write(
                x,
                y,
                Rgba8 {
                    r: 16,
                    g: 0,
                    b: 0,
                    a: 255,
                },
            );
        }
        let cmp = compare_images(&fb1, &fb2);
        assert_eq!(cmp.verdict, OracleVerdict::Fail);
    }
}
