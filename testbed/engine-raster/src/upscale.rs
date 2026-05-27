//! CPU bilinear-upscale oracle (Phase 5 PR 5, ADR-005).
//!
//! `engine_render::upscale::OwnedBilinear` is the Phase-5 placeholder
//! the trait surface ships. The actual pixel math lives here — pure
//! CPU, std-only, deterministic — so the oracle and the frame-pacing
//! milestone bench share a single reference. The Phase 6 owned ONNX
//! temporal upscaler will land its own oracle alongside per ADR-005
//! §Verification.
//!
//! The classic bilinear formula: for output pixel `(x, y)` at
//! resolution `(dst_w, dst_h)` from source `(src_w, src_h)`, the
//! source coordinate is
//!
//! ```text
//! sx = (x + 0.5) * src_w / dst_w − 0.5
//! sy = (y + 0.5) * src_h / dst_h − 0.5
//! ```
//!
//! and the output is the bilinear blend of the four surrounding source
//! pixels. Out-of-bounds source coordinates clamp to the edge (border
//! pixels are duplicated rather than wrapped or zeroed).
//!
//! Determinism: every operation uses `+ − × ÷`. No `powf` / `sin` /
//! `cos`. The result is bit-stable across hosts under the determinism
//! contract (ADR-013).

use engine_math::Vec3;

/// Output dimensions for the milestone-bench upscale (ADR-053 PR-5).
/// 1440p — the Phase-5 RX-580 spec target. The bench uses these as
/// defaults; tests use smaller resolutions for speed.
pub const MILESTONE_OUTPUT_WIDTH: u32 = 2560;
/// See [`MILESTONE_OUTPUT_WIDTH`].
pub const MILESTONE_OUTPUT_HEIGHT: u32 = 1440;
/// Internal render width the bench upscales from (`scale = 0.5` →
/// 1280×720 in → 2560×1440 out). The owned-bilinear placeholder is
/// not quality-competitive with vendor upscalers; the Phase 5
/// milestone is the *trait surface* end-to-end, not the perceptual
/// score.
pub const MILESTONE_INPUT_WIDTH: u32 = 1280;
/// See [`MILESTONE_INPUT_WIDTH`].
pub const MILESTONE_INPUT_HEIGHT: u32 = 720;

/// Sample `src` at integer `(x, y)`, clamping to the bounds.
#[inline]
fn sample_clamp(src: &[Vec3], src_w: u32, src_h: u32, x: i32, y: i32) -> Vec3 {
    let cx = x.clamp(0, src_w as i32 - 1) as u32;
    let cy = y.clamp(0, src_h as i32 - 1) as u32;
    src[(cy as usize) * (src_w as usize) + (cx as usize)]
}

/// Bilinearly upscale `src` (`src_w × src_h`) into `dst` (`dst_w × dst_h`).
///
/// Panics if `src.len() != src_w * src_h` or `dst.len() != dst_w * dst_h`.
/// Source and destination dimensions must each be ≥ 1.
pub fn bilinear_upscale(
    src: &[Vec3],
    src_w: u32,
    src_h: u32,
    dst: &mut [Vec3],
    dst_w: u32,
    dst_h: u32,
) {
    assert!(
        src_w > 0 && src_h > 0,
        "bilinear_upscale: zero source extent"
    );
    assert!(dst_w > 0 && dst_h > 0, "bilinear_upscale: zero dest extent");
    assert_eq!(
        src.len(),
        (src_w as usize) * (src_h as usize),
        "bilinear_upscale: src length mismatch"
    );
    assert_eq!(
        dst.len(),
        (dst_w as usize) * (dst_h as usize),
        "bilinear_upscale: dst length mismatch"
    );

    let sx_scale = src_w as f32 / dst_w as f32;
    let sy_scale = src_h as f32 / dst_h as f32;

    for dy in 0..dst_h {
        let sy = (dy as f32 + 0.5) * sy_scale - 0.5;
        let y0 = sy.floor() as i32;
        let y1 = y0 + 1;
        let fy = sy - y0 as f32;

        for dx in 0..dst_w {
            let sx = (dx as f32 + 0.5) * sx_scale - 0.5;
            let x0 = sx.floor() as i32;
            let x1 = x0 + 1;
            let fx = sx - x0 as f32;

            let p00 = sample_clamp(src, src_w, src_h, x0, y0);
            let p10 = sample_clamp(src, src_w, src_h, x1, y0);
            let p01 = sample_clamp(src, src_w, src_h, x0, y1);
            let p11 = sample_clamp(src, src_w, src_h, x1, y1);

            // Bilinear: top row blend, bottom row blend, then vertical
            // blend. Per-channel `(1-f)·a + f·b` is monotone in `f`.
            let top = Vec3::new(
                p00.x * (1.0 - fx) + p10.x * fx,
                p00.y * (1.0 - fx) + p10.y * fx,
                p00.z * (1.0 - fx) + p10.z * fx,
            );
            let bot = Vec3::new(
                p01.x * (1.0 - fx) + p11.x * fx,
                p01.y * (1.0 - fx) + p11.y * fx,
                p01.z * (1.0 - fx) + p11.z * fx,
            );
            let out = Vec3::new(
                top.x * (1.0 - fy) + bot.x * fy,
                top.y * (1.0 - fy) + bot.y * fy,
                top.z * (1.0 - fy) + bot.z * fy,
            );
            dst[(dy as usize) * (dst_w as usize) + (dx as usize)] = out;
        }
    }
}

/// Allocating convenience: returns a freshly-sized output buffer.
pub fn bilinear_upscale_to_vec(
    src: &[Vec3],
    src_w: u32,
    src_h: u32,
    dst_w: u32,
    dst_h: u32,
) -> Vec<Vec3> {
    let mut dst = vec![Vec3::ZERO; (dst_w as usize) * (dst_h as usize)];
    bilinear_upscale(src, src_w, src_h, &mut dst, dst_w, dst_h);
    dst
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(x: f32, y: f32, z: f32) -> Vec3 {
        Vec3::new(x, y, z)
    }

    #[test]
    fn identity_one_to_one_preserves_pixels() {
        // Upscaling a buffer to its own resolution must return the
        // original (centre-aligned sampling lands exactly on each
        // source pixel).
        let src = vec![
            v(0.1, 0.2, 0.3),
            v(0.4, 0.5, 0.6),
            v(0.7, 0.8, 0.9),
            v(1.0, 0.0, 0.5),
        ];
        let dst = bilinear_upscale_to_vec(&src, 2, 2, 2, 2);
        for (a, b) in src.iter().zip(dst.iter()) {
            assert!((a.x - b.x).abs() < 1e-6, "got {b:?} want {a:?}");
            assert!((a.y - b.y).abs() < 1e-6);
            assert!((a.z - b.z).abs() < 1e-6);
        }
    }

    #[test]
    fn constant_field_is_invariant() {
        let src = vec![v(0.3, 0.7, 0.1); 16]; // 4×4 constant
        let dst = bilinear_upscale_to_vec(&src, 4, 4, 8, 8);
        for p in &dst {
            assert!((p.x - 0.3).abs() < 1e-6, "got {p:?}");
            assert!((p.y - 0.7).abs() < 1e-6);
            assert!((p.z - 0.1).abs() < 1e-6);
        }
    }

    #[test]
    fn two_by_two_upscaled_to_four_by_four_blends_corners() {
        // 2×2 with distinct corners. The 4×4 output's corner pixels
        // sample near the source corners (sx = -0.25, mapped to 0
        // by clamp); the centre pixels are blended.
        let src = vec![
            v(1.0, 0.0, 0.0),
            v(0.0, 1.0, 0.0),
            v(0.0, 0.0, 1.0),
            v(1.0, 1.0, 1.0),
        ];
        let dst = bilinear_upscale_to_vec(&src, 2, 2, 4, 4);
        // Top-left output ≈ source top-left.
        let tl = dst[0];
        assert!(tl.x > 0.9, "top-left should be ~red: {tl:?}");
        // Bottom-right output ≈ source bottom-right.
        let br = dst[15];
        assert!(br.x > 0.9 && br.y > 0.9 && br.z > 0.9, "br: {br:?}");
        // Centre output blends all four corners — every channel is
        // strictly between 0 and 1.
        let centre = dst[(2 * 4) + 2];
        for c in [centre.x, centre.y, centre.z] {
            assert!(c > 0.0 && c < 1.0, "centre channel: {c}");
        }
    }

    #[test]
    fn monotone_horizontal_gradient_preserves_monotone() {
        // Source is a 4×1 horizontal gradient. Upscaled output must
        // remain non-decreasing left-to-right (bilinear is linear in
        // each axis and the gradient is monotonic).
        let src = vec![
            v(0.0, 0.0, 0.0),
            v(0.25, 0.0, 0.0),
            v(0.5, 0.0, 0.0),
            v(1.0, 0.0, 0.0),
        ];
        let dst = bilinear_upscale_to_vec(&src, 4, 1, 12, 1);
        for w in dst.windows(2) {
            assert!(w[1].x + 1e-6 >= w[0].x, "non-monotone: {:?}", dst);
        }
    }

    #[test]
    fn output_extent_constants_match_milestone() {
        // ADR-053 PR-5: 1440p output, 0.5x internal scale → 720p input.
        assert_eq!(MILESTONE_OUTPUT_WIDTH, 2560);
        assert_eq!(MILESTONE_OUTPUT_HEIGHT, 1440);
        assert_eq!(MILESTONE_INPUT_WIDTH, 1280);
        assert_eq!(MILESTONE_INPUT_HEIGHT, 720);
        assert_eq!(MILESTONE_OUTPUT_WIDTH, MILESTONE_INPUT_WIDTH * 2);
        assert_eq!(MILESTONE_OUTPUT_HEIGHT, MILESTONE_INPUT_HEIGHT * 2);
    }

    #[test]
    fn deterministic_across_two_runs() {
        // Same input + same dimensions → byte-identical output (under
        // the determinism contract; ADR-013).
        let src: Vec<Vec3> = (0..16)
            .map(|i| {
                let f = i as f32 / 15.0;
                v(f, 1.0 - f, f * f)
            })
            .collect();
        let a = bilinear_upscale_to_vec(&src, 4, 4, 11, 7);
        let b = bilinear_upscale_to_vec(&src, 4, 4, 11, 7);
        assert_eq!(a, b, "two runs of bilinear_upscale must match exactly");
    }
}
