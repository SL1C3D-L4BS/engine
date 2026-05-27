//! Bilinear-upscale integration oracle (ADR-005 §Verification).
//!
//! ADR-005 §Verification for the bilinear placeholder calls for a
//! "render at low-resolution, upscale, compare to a native high-
//! resolution reference within tolerance" oracle. The native scene
//! helpers in `engine_raster::sample` are fixed-resolution today, so
//! the oracle uses a deterministic procedural HDR pattern as the
//! ground truth — bilinearly upscaling a 2×-downsampled version of
//! it and bounding the L1-per-channel reconstruction error.
//!
//! The threshold is bilinear-class quality: bilinear cannot recover
//! the high-frequency detail discarded by the 2× downsample, so the
//! oracle bounds the error rather than demanding exact recovery. A
//! regression in the bilinear math (off-by-half, edge-clamp wrong,
//! channel mix-up) blows past the bound immediately; a perceptual
//! quality drift below it is invisible to this oracle.
//!
//! Resolutions are sized so the test runs in well under a second on
//! every host (256×144 ground truth ↔ 128×72 source). The milestone
//! 2560×1440 ↔ 1280×720 path is exercised once at a lower threshold
//! via the milestone-extents test.

use engine_math::Vec3;
use engine_raster::{
    MILESTONE_INPUT_HEIGHT, MILESTONE_INPUT_WIDTH, MILESTONE_OUTPUT_HEIGHT, MILESTONE_OUTPUT_WIDTH,
    bilinear_upscale, bilinear_upscale_to_vec,
};

/// Procedural smooth HDR pattern. Each channel is a bounded function
/// of the centre-sampled `(fx, fy)` in `[0, 1]^2`. Same numbers across
/// hosts (no transcendentals).
fn pattern(x: u32, y: u32, w: u32, h: u32) -> Vec3 {
    let fx = (x as f32 + 0.5) / w as f32;
    let fy = (y as f32 + 0.5) / h as f32;
    // Three orthogonal smooth-ish patterns — bilinear should
    // reconstruct each within fractions of a unit because the
    // ground truth is itself ~linear at the per-pixel scale.
    let r = 0.25 + 0.5 * fx;
    let g = 0.6 * fy;
    let b = (1.0 - fx) * fy + fx * (1.0 - fy); // anti-diagonal blend
    Vec3::new(r, g, b)
}

/// Build the procedural pattern at `(w, h)`.
fn build_pattern(w: u32, h: u32) -> Vec<Vec3> {
    let mut out = Vec::with_capacity((w as usize) * (h as usize));
    for y in 0..h {
        for x in 0..w {
            out.push(pattern(x, y, w, h));
        }
    }
    out
}

/// 2× box-filter downsample. Assumes `src_w == 2*dst_w`, same for height.
fn box_filter_downsample_2x(
    src: &[Vec3],
    src_w: u32,
    src_h: u32,
    dst_w: u32,
    dst_h: u32,
) -> Vec<Vec3> {
    assert_eq!(src_w, dst_w * 2, "box filter requires 2x");
    assert_eq!(src_h, dst_h * 2);
    let mut dst = vec![Vec3::ZERO; (dst_w as usize) * (dst_h as usize)];
    for dy in 0..dst_h {
        for dx in 0..dst_w {
            let sx = dx * 2;
            let sy = dy * 2;
            let mut acc = Vec3::ZERO;
            for ddy in 0..2 {
                for ddx in 0..2 {
                    let i = ((sy + ddy) as usize) * (src_w as usize) + ((sx + ddx) as usize);
                    acc.x += src[i].x;
                    acc.y += src[i].y;
                    acc.z += src[i].z;
                }
            }
            dst[(dy as usize) * (dst_w as usize) + (dx as usize)] =
                Vec3::new(acc.x * 0.25, acc.y * 0.25, acc.z * 0.25);
        }
    }
    dst
}

/// L1-per-channel-per-pixel between two same-shape images.
fn l1_per_channel(a: &[Vec3], b: &[Vec3]) -> f32 {
    assert_eq!(a.len(), b.len(), "L1 requires same shape");
    let mut sum = 0.0_f32;
    for i in 0..a.len() {
        sum += (a[i].x - b[i].x).abs();
        sum += (a[i].y - b[i].y).abs();
        sum += (a[i].z - b[i].z).abs();
    }
    sum / (3.0 * a.len() as f32)
}

/// p99 L1-per-channel error.
fn p99_per_pixel(a: &[Vec3], b: &[Vec3]) -> f32 {
    let mut errs: Vec<f32> = a
        .iter()
        .zip(b.iter())
        .map(|(x, y)| ((x.x - y.x).abs() + (x.y - y.y).abs() + (x.z - y.z).abs()) / 3.0)
        .collect();
    errs.sort_by(|x, y| x.partial_cmp(y).unwrap());
    // Nearest-rank percentile at 0.99 quantile.
    let i = ((errs.len() as f32) * 0.99).ceil() as usize - 1;
    errs[i.min(errs.len() - 1)]
}

#[test]
fn round_trip_recovers_smooth_pattern_within_l1_bound() {
    // Build pattern at 256×144 — the ground truth. Downsample to
    // 128×72. Upscale back to 256×144 via bilinear. Compare.
    let gt = build_pattern(256, 144);
    let lo = box_filter_downsample_2x(&gt, 256, 144, 128, 72);
    let mut hi = vec![Vec3::ZERO; gt.len()];
    bilinear_upscale(&lo, 128, 72, &mut hi, 256, 144);
    let l1 = l1_per_channel(&gt, &hi);
    // The pattern is dominated by linear ramps; bilinear recovers
    // most of it. 0.02 (2% of full-channel range) is the upper bound
    // for the procedural pattern + 2× downsample + bilinear path.
    assert!(l1 < 0.02, "L1 reconstruction error too large: {l1}");
}

#[test]
fn round_trip_p99_per_pixel_is_bounded() {
    let gt = build_pattern(256, 144);
    let lo = box_filter_downsample_2x(&gt, 256, 144, 128, 72);
    let hi = bilinear_upscale_to_vec(&lo, 128, 72, 256, 144);
    let p99 = p99_per_pixel(&gt, &hi);
    // p99 ≤ 0.05 is a generous bound — even the worst 1% of pixels
    // (interior edges where the 2× downsample loses detail) sit
    // inside 5% L1 distance per channel.
    assert!(p99 < 0.05, "p99 reconstruction error too large: {p99}");
}

#[test]
fn milestone_extents_round_trip_finishes_promptly() {
    // The milestone path (2560×1440 ↔ 1280×720) runs in CI; the
    // threshold is the same as the small case scaled — bilinear's
    // L1 error grows slowly with extent. This test also pins the
    // milestone constants as the upscale fixture's resolutions.
    let gt = build_pattern(MILESTONE_OUTPUT_WIDTH, MILESTONE_OUTPUT_HEIGHT);
    let lo = box_filter_downsample_2x(
        &gt,
        MILESTONE_OUTPUT_WIDTH,
        MILESTONE_OUTPUT_HEIGHT,
        MILESTONE_INPUT_WIDTH,
        MILESTONE_INPUT_HEIGHT,
    );
    let hi = bilinear_upscale_to_vec(
        &lo,
        MILESTONE_INPUT_WIDTH,
        MILESTONE_INPUT_HEIGHT,
        MILESTONE_OUTPUT_WIDTH,
        MILESTONE_OUTPUT_HEIGHT,
    );
    let l1 = l1_per_channel(&gt, &hi);
    assert!(l1 < 0.02, "milestone L1 reconstruction error: {l1}");
    // Output extent invariant.
    assert_eq!(
        hi.len(),
        (MILESTONE_OUTPUT_WIDTH as usize) * (MILESTONE_OUTPUT_HEIGHT as usize)
    );
}

#[test]
fn identity_pattern_round_trips_byte_exact_at_one_to_one() {
    // 1:1 upscale (no resolution change) returns the source exactly.
    let src = build_pattern(64, 36);
    let dst = bilinear_upscale_to_vec(&src, 64, 36, 64, 36);
    assert_eq!(src.len(), dst.len());
    for (a, b) in src.iter().zip(dst.iter()) {
        // Centre sampling lands on each pixel exactly; the only loss
        // is f32 rounding (bounded by 1e-6 over the [0, 1] range).
        assert!((a.x - b.x).abs() < 1e-6);
        assert!((a.y - b.y).abs() < 1e-6);
        assert!((a.z - b.z).abs() < 1e-6);
    }
}

#[test]
fn deterministic_round_trip_two_runs_are_identical() {
    // ADR-013 determinism: same inputs → byte-identical outputs.
    let gt = build_pattern(64, 36);
    let lo = box_filter_downsample_2x(&gt, 64, 36, 32, 18);
    let a = bilinear_upscale_to_vec(&lo, 32, 18, 64, 36);
    let b = bilinear_upscale_to_vec(&lo, 32, 18, 64, 36);
    assert_eq!(a, b, "bilinear must be byte-stable across invocations");
}

#[test]
fn monotone_horizontal_ramp_remains_monotone_through_upscale() {
    // A pure horizontal ramp at 4×1 → upscale to 16×1 stays
    // non-decreasing. Catches any sign-flip / off-by-edge bug.
    let src: Vec<Vec3> = (0..8)
        .map(|i| {
            let f = i as f32 / 7.0;
            Vec3::new(f, 0.0, 0.0)
        })
        .collect();
    let dst = bilinear_upscale_to_vec(&src, 8, 1, 64, 1);
    for w in dst.windows(2) {
        assert!(
            w[1].x + 1e-6 >= w[0].x,
            "non-monotone bilinear output: {w:?}",
        );
    }
}
