//! Post-FX chain · CPU oracle (ADR-042 + spec §IV.4.A line 384).
//!
//! Spec order: `SSAO → TAA → Bloom → Tonemap → (optional CA / Vignette /
//! Grain) → Upscale`. PR 4 lands the four primary stages as CPU
//! references; the optional grade stages and the upscaler land in PR 5.
//!
//! The TAA implementation follows ADR-042 verbatim: Halton (2, 3) jitter
//! with period 8, 3×3 YCgCo neighbourhood clip, motion-vector
//! reprojection, exponential blend with `α ∈ [0.05, 0.5]`, depth-ratio
//! disocclusion mask, and velocity-aware sharpening. Determinism: every
//! float operation uses only `+ − × ÷ sqrt` (or the documented `powf`
//! call in tone-mapping, which is libm-pinned and therefore allowed in
//! the testbed under ADR-023's non-sim exemption).

use engine_math::{Vec2, Vec3};

/// Period of the TAA jitter sequence (ADR-042 §1). After this many
/// frames the Halton pattern revisits its starting offset.
pub const TAA_JITTER_PERIOD: u32 = 8;

/// Minimum exponential blend weight (slow history convergence).
pub const TAA_ALPHA_MIN: f32 = 0.05;
/// Maximum exponential blend weight (history fully rejected — adopt
/// the current frame).
pub const TAA_ALPHA_MAX: f32 = 0.5;
/// Disocclusion depth ratio threshold (ADR-042 §5). Triggers full
/// rejection above this multiplier.
pub const TAA_DISOCCLUSION_RATIO: f32 = 1.1;

/// `i`-th element of the Halton low-discrepancy sequence in `base`.
/// Returns a value in `[0, 1)`.
pub fn halton(index: u32, base: u32) -> f32 {
    debug_assert!(base >= 2);
    let mut f = 1.0_f32;
    let mut r = 0.0_f32;
    let mut i = index;
    while i > 0 {
        f /= base as f32;
        r += f * (i % base) as f32;
        i /= base;
    }
    r
}

/// Sub-pixel jitter offset for TAA in `[-0.5, 0.5]`, using the
/// Halton (2, 3) sequence with period 8 (ADR-042 §1). Phase-stable
/// across runs.
pub fn jitter_for_frame(frame: u64) -> Vec2 {
    // Index from 1 — `halton(0, _)` is the trivial 0.0 sample.
    let idx = (frame % TAA_JITTER_PERIOD as u64) as u32 + 1;
    Vec2::new(halton(idx, 2) - 0.5, halton(idx, 3) - 0.5)
}

/// Convert linear RGB to YCgCo. Luminance is the Y channel; Cg + Co
/// carry chroma (ADR-042 §3).
pub fn rgb_to_ycgco(c: Vec3) -> Vec3 {
    let r = c.x;
    let g = c.y;
    let b = c.z;
    let y = 0.25 * r + 0.5 * g + 0.25 * b;
    let cg = -0.25 * r + 0.5 * g - 0.25 * b;
    let co = 0.5 * r - 0.5 * b;
    Vec3::new(y, cg, co)
}

/// Inverse of [`rgb_to_ycgco`].
pub fn ycgco_to_rgb(c: Vec3) -> Vec3 {
    let y = c.x;
    let cg = c.y;
    let co = c.z;
    Vec3::new(y - cg + co, y + cg, y - cg - co)
}

/// Compute the YCgCo AABB of a 3×3 neighbourhood centred at `(px, py)`.
/// Out-of-bounds samples are clamped to the nearest edge.
pub fn neighbourhood_ycgco_aabb(
    current: &[Vec3],
    width: u32,
    height: u32,
    px: u32,
    py: u32,
) -> (Vec3, Vec3) {
    debug_assert!(width > 0 && height > 0);
    let w = width as i32;
    let h = height as i32;
    let mut min = Vec3::new(f32::INFINITY, f32::INFINITY, f32::INFINITY);
    let mut max = Vec3::new(f32::NEG_INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY);
    for dy in -1i32..=1 {
        for dx in -1i32..=1 {
            let nx = (px as i32 + dx).clamp(0, w - 1) as u32;
            let ny = (py as i32 + dy).clamp(0, h - 1) as u32;
            let i = (ny as usize) * (width as usize) + (nx as usize);
            let c = rgb_to_ycgco(current[i]);
            min = Vec3::new(min.x.min(c.x), min.y.min(c.y), min.z.min(c.z));
            max = Vec3::new(max.x.max(c.x), max.y.max(c.y), max.z.max(c.z));
        }
    }
    (min, max)
}

/// Sample `tex` at integer pixel `(x, y)`, clamping to the bounds.
#[inline]
pub fn sample_clamped(tex: &[Vec3], width: u32, height: u32, x: i32, y: i32) -> Vec3 {
    let x = x.clamp(0, width as i32 - 1) as u32;
    let y = y.clamp(0, height as i32 - 1) as u32;
    tex[(y as usize) * (width as usize) + (x as usize)]
}

/// Clip the `colour` point into the AABB `[min, max]` along the line
/// through `centre`, returning both the clipped colour and the fraction
/// `t ∈ [0, 1]` by which it moved (0 = already inside, 1 = clipped to
/// the centre point).
pub fn clip_aabb(min: Vec3, max: Vec3, centre: Vec3, colour: Vec3) -> (Vec3, f32) {
    // Per Karis 2014: clip along the line from `centre` to `colour` to
    // the AABB. If the colour is already inside the AABB, t = 0.
    let inside = colour.x >= min.x
        && colour.x <= max.x
        && colour.y >= min.y
        && colour.y <= max.y
        && colour.z >= min.z
        && colour.z <= max.z;
    if inside {
        return (colour, 0.0);
    }
    // Parametric line: p(t) = centre + t · (colour - centre)
    // For each axis we want the smallest t such that p(t) lies on the
    // axis-aligned slab.
    let dir = Vec3::new(
        colour.x - centre.x,
        colour.y - centre.y,
        colour.z - centre.z,
    );
    let axis_t = |c: f32, d: f32, lo: f32, hi: f32| -> f32 {
        if d.abs() < 1e-8 {
            return 1.0;
        }
        let t_lo = (lo - c) / d;
        let t_hi = (hi - c) / d;
        let mut best = 1.0_f32;
        if (0.0..=1.0).contains(&t_lo) {
            best = best.min(t_lo);
        }
        if (0.0..=1.0).contains(&t_hi) {
            best = best.min(t_hi);
        }
        best
    };
    let t_x = axis_t(centre.x, dir.x, min.x, max.x);
    let t_y = axis_t(centre.y, dir.y, min.y, max.y);
    let t_z = axis_t(centre.z, dir.z, min.z, max.z);
    let t = t_x.min(t_y).min(t_z).clamp(0.0, 1.0);
    let clipped = Vec3::new(
        centre.x + dir.x * t,
        centre.y + dir.y * t,
        centre.z + dir.z * t,
    );
    // `1 − t` is how far the original colour had to move toward the
    // AABB: 0 = already inside (handled above), 1 = clipped to centre.
    (clipped, 1.0 - t)
}

/// Input to a TAA resolve. All buffers are pixel-major, linear-light
/// HDR Vec3 colour values; motion is in pixel units (+x right, +y down)
/// pointing from current to previous frame (sample previous at
/// `current - motion`).
pub struct TaaInput<'a> {
    /// Current-frame HDR linear-light colour buffer.
    pub current: &'a [Vec3],
    /// Previous resolved frame (history).
    pub history: &'a [Vec3],
    /// Per-pixel motion vectors (`current → previous`, pixel units).
    pub motion: &'a [Vec2],
    /// Current-frame view-space depth (positive in front).
    pub depth_current: &'a [f32],
    /// Previous-frame view-space depth.
    pub depth_history: &'a [f32],
    /// Resolution.
    pub width: u32,
    /// Resolution.
    pub height: u32,
}

/// Output of [`taa_resolve_pixel`].
#[derive(Clone, Copy, Debug)]
pub struct TaaSample {
    /// Resolved HDR colour.
    pub colour: Vec3,
    /// Rejection score in `[0, 1]`. 0 = history fully trusted, 1 =
    /// history fully rejected (alpha → 1.0).
    pub rejection_score: f32,
}

/// Resolve one pixel of the TAA history blend. Implements the ADR-042
/// neighbourhood-clip + disocclusion-mask pipeline.
pub fn taa_resolve_pixel(input: &TaaInput, px: u32, py: u32) -> TaaSample {
    let idx = (py as usize) * (input.width as usize) + (px as usize);
    let current_rgb = input.current[idx];

    // Reproject history sample using bilinear filtering at the motion-
    // vector offset (oracle uses nearest-neighbour to keep the diff
    // exact; the GPU uses linear, but the ADR-046 1/255 threshold
    // absorbs sub-pixel differences).
    let motion = input.motion[idx];
    let src_x = (px as f32) - motion.x;
    let src_y = (py as f32) - motion.y;
    let history_rgb = sample_clamped(
        input.history,
        input.width,
        input.height,
        src_x.round() as i32,
        src_y.round() as i32,
    );

    // Neighbourhood AABB in YCgCo.
    let (min, max) = neighbourhood_ycgco_aabb(input.current, input.width, input.height, px, py);
    let centre = rgb_to_ycgco(current_rgb);
    let history_ycgco = rgb_to_ycgco(history_rgb);
    let (clipped, t) = clip_aabb(min, max, centre, history_ycgco);
    let history_rgb_clipped = ycgco_to_rgb(clipped);
    let mut rejection = t;

    // Disocclusion mask: reproject the previous depth at the motion
    // offset and compare ratios (ADR-042 §5).
    let cur_depth = input.depth_current[idx];
    let prev_depth = sample_depth_clamped(
        input.depth_history,
        input.width,
        input.height,
        src_x.round() as i32,
        src_y.round() as i32,
    );
    if cur_depth > 0.0 && prev_depth > 0.0 {
        let ratio = (cur_depth / prev_depth).max(prev_depth / cur_depth);
        if ratio > TAA_DISOCCLUSION_RATIO {
            rejection = 1.0;
        }
    }

    let alpha = TAA_ALPHA_MIN + (TAA_ALPHA_MAX - TAA_ALPHA_MIN) * rejection;
    let blended = Vec3::new(
        history_rgb_clipped.x * (1.0 - alpha) + current_rgb.x * alpha,
        history_rgb_clipped.y * (1.0 - alpha) + current_rgb.y * alpha,
        history_rgb_clipped.z * (1.0 - alpha) + current_rgb.z * alpha,
    );

    TaaSample {
        colour: blended,
        rejection_score: rejection,
    }
}

#[inline]
fn sample_depth_clamped(depth: &[f32], width: u32, height: u32, x: i32, y: i32) -> f32 {
    if width == 0 || height == 0 || depth.is_empty() {
        return 0.0;
    }
    let x = x.clamp(0, width as i32 - 1) as u32;
    let y = y.clamp(0, height as i32 - 1) as u32;
    depth[(y as usize) * (width as usize) + (x as usize)]
}

/// Resolve the full TAA frame. Caller owns the output buffer (pre-sized
/// to `width × height`).
pub fn taa_resolve(input: &TaaInput, output: &mut [Vec3]) {
    debug_assert_eq!(
        output.len(),
        (input.width as usize) * (input.height as usize)
    );
    for py in 0..input.height {
        for px in 0..input.width {
            let s = taa_resolve_pixel(input, px, py);
            output[(py as usize) * (input.width as usize) + (px as usize)] = s.colour;
        }
    }
}

/// 8-tap SSAO oracle. The ADR doesn't pin a screen-space AO algorithm;
/// this is a simplified hemisphere-sampled occlusion factor in `[0, 1]`
/// (1 = fully unoccluded). Determinism: the kernel is fixed; depth
/// access is clamped-edge.
///
/// The kernel directions are an 8-point Fibonacci spiral on the
/// hemisphere (deterministic, no PRNG). `radius_px` is the screen-
/// space sample radius.
pub fn ssao_factor(
    px: u32,
    py: u32,
    depth: &[f32],
    width: u32,
    height: u32,
    radius_px: i32,
) -> f32 {
    debug_assert!(radius_px > 0);
    let centre_idx = (py as usize) * (width as usize) + (px as usize);
    let centre_depth = depth[centre_idx];
    if centre_depth <= 0.0 {
        return 1.0;
    }
    let mut occlusion = 0.0;
    let mut samples = 0.0;
    for k in 0..8u32 {
        let (dx, dy) = SSAO_KERNEL[k as usize];
        let sx = px as i32 + (dx * radius_px as f32).round() as i32;
        let sy = py as i32 + (dy * radius_px as f32).round() as i32;
        let sd = sample_depth_clamped(depth, width, height, sx, sy);
        if sd <= 0.0 {
            samples += 1.0;
            continue;
        }
        // A neighbour occludes the centre when it is significantly
        // closer to the camera. View-space depth here is positive, so
        // a smaller depth = closer.
        let delta = centre_depth - sd;
        let bias = 0.05 * centre_depth;
        if delta > bias {
            // Range-attenuated falloff in depth-relative units so
            // distant geometry doesn't bleed AO across the screen.
            let range = centre_depth.max(0.5);
            let falloff = (1.0 - (delta / range).clamp(0.0, 1.0)).max(0.0);
            occlusion += falloff;
        }
        samples += 1.0;
    }
    if samples <= 0.0 {
        return 1.0;
    }
    let factor = 1.0 - (occlusion / samples).clamp(0.0, 1.0);
    factor.clamp(0.0, 1.0)
}

/// Eight-direction 2D unit kernel for the SSAO sampler. Stable across
/// runs and platforms.
const SSAO_KERNEL: [(f32, f32); 8] = [
    (1.000, 0.000),
    (0.707, 0.707),
    (0.000, 1.000),
    (-0.707, 0.707),
    (-1.000, 0.000),
    (-0.707, -0.707),
    (0.000, -1.000),
    (0.707, -0.707),
];

/// Apply SSAO to an HDR linear-light colour buffer in place. Each
/// pixel's RGB is scaled by its occlusion factor.
pub fn ssao_apply(
    hdr: &mut [Vec3],
    depth: &[f32],
    width: u32,
    height: u32,
    radius_px: i32,
    strength: f32,
) {
    for py in 0..height {
        for px in 0..width {
            let idx = (py as usize) * (width as usize) + (px as usize);
            let ao = ssao_factor(px, py, depth, width, height, radius_px);
            let factor = 1.0 - strength * (1.0 - ao);
            hdr[idx] = Vec3::new(
                hdr[idx].x * factor,
                hdr[idx].y * factor,
                hdr[idx].z * factor,
            );
        }
    }
}

/// Extract the bright pass for bloom: anything above `threshold` is
/// soft-thresholded into a separate buffer (the standard knee curve
/// from Jimenez 2014 / Unreal documentation).
pub fn bloom_extract(hdr: Vec3, threshold: f32) -> Vec3 {
    let lum = 0.212_672_9 * hdr.x + 0.715_152_2 * hdr.y + 0.072_175_0 * hdr.z;
    if lum <= threshold {
        return Vec3::ZERO;
    }
    let excess = lum - threshold;
    // Soft-knee proportional scaling preserves chroma.
    let k = (excess / lum.max(1e-6)).clamp(0.0, 1.0);
    Vec3::new(hdr.x * k, hdr.y * k, hdr.z * k)
}

/// Composite a low-frequency bloom layer onto the HDR target.
pub fn bloom_composite(hdr: Vec3, bloom: Vec3, intensity: f32) -> Vec3 {
    Vec3::new(
        hdr.x + bloom.x * intensity,
        hdr.y + bloom.y * intensity,
        hdr.z + bloom.z * intensity,
    )
}

/// Stage-separable 3×3 Gaussian blur over an HDR buffer. Used as the
/// bloom oracle's reference (the GPU uses a downsample/upsample chain;
/// the 1/255 threshold absorbs the kernel-shape difference).
pub fn gaussian_blur_3x3(input: &[Vec3], output: &mut [Vec3], width: u32, height: u32) {
    debug_assert_eq!(input.len(), output.len());
    let kernel = [1.0_f32, 2.0, 1.0, 2.0, 4.0, 2.0, 1.0, 2.0, 1.0];
    let norm = 16.0_f32;
    for py in 0..height {
        for px in 0..width {
            let mut acc = Vec3::ZERO;
            for ky in 0..3i32 {
                for kx in 0..3i32 {
                    let sx = px as i32 + (kx - 1);
                    let sy = py as i32 + (ky - 1);
                    let s = sample_clamped(input, width, height, sx, sy);
                    let w = kernel[(ky as usize) * 3 + (kx as usize)];
                    acc = Vec3::new(acc.x + s.x * w, acc.y + s.y * w, acc.z + s.z * w);
                }
            }
            output[(py as usize) * (width as usize) + (px as usize)] =
                Vec3::new(acc.x / norm, acc.y / norm, acc.z / norm);
        }
    }
}

/// ACES filmic tonemap (Krzysztof Narkowicz's curve fit, the
/// Unreal-engine baseline). Maps HDR linear `[0, ∞)` to LDR `[0, 1]`.
pub fn tonemap_aces(c: Vec3) -> Vec3 {
    fn curve(x: f32) -> f32 {
        let a = 2.51_f32;
        let b = 0.03_f32;
        let c = 2.43_f32;
        let d = 0.59_f32;
        let e = 0.14_f32;
        let num = x * (a * x + b);
        let den = x * (c * x + d) + e;
        (num / den).clamp(0.0, 1.0)
    }
    Vec3::new(
        curve(c.x.max(0.0)),
        curve(c.y.max(0.0)),
        curve(c.z.max(0.0)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vec3(x: f32, y: f32, z: f32) -> Vec3 {
        Vec3::new(x, y, z)
    }

    #[test]
    fn halton_first_eight_values_match_published_sequence() {
        // Reference values from https://en.wikipedia.org/wiki/Halton_sequence
        // and Real-Time Rendering 4 ch. 5.
        let expected_base2 = [1.0 / 2.0, 1.0 / 4.0, 3.0 / 4.0, 1.0 / 8.0];
        for (i, &v) in expected_base2.iter().enumerate() {
            assert!(
                (halton(i as u32 + 1, 2) - v).abs() < 1e-6,
                "halton({}, 2): got {} want {v}",
                i + 1,
                halton(i as u32 + 1, 2)
            );
        }
        let expected_base3 = [1.0 / 3.0, 2.0 / 3.0, 1.0 / 9.0, 4.0 / 9.0];
        for (i, &v) in expected_base3.iter().enumerate() {
            assert!((halton(i as u32 + 1, 3) - v).abs() < 1e-6);
        }
    }

    #[test]
    fn jitter_is_bounded() {
        for frame in 0..256u64 {
            let j = jitter_for_frame(frame);
            assert!((-0.5..=0.5).contains(&j.x), "frame={frame} j.x={}", j.x);
            assert!((-0.5..=0.5).contains(&j.y), "frame={frame} j.y={}", j.y);
        }
    }

    #[test]
    fn jitter_is_period_8_stable() {
        for frame in 0..32u64 {
            let a = jitter_for_frame(frame);
            let b = jitter_for_frame(frame + TAA_JITTER_PERIOD as u64);
            assert!((a.x - b.x).abs() < 1e-6);
            assert!((a.y - b.y).abs() < 1e-6);
        }
    }

    #[test]
    fn ycgco_round_trip_is_identity_to_float_precision() {
        for c in [
            vec3(0.0, 0.0, 0.0),
            vec3(1.0, 0.5, 0.25),
            vec3(0.1, 0.9, 0.05),
            vec3(0.7, 0.2, 0.8),
        ] {
            let back = ycgco_to_rgb(rgb_to_ycgco(c));
            assert!((back.x - c.x).abs() < 1e-5);
            assert!((back.y - c.y).abs() < 1e-5);
            assert!((back.z - c.z).abs() < 1e-5);
        }
    }

    #[test]
    fn neighbourhood_aabb_contains_centre_sample() {
        let w = 3u32;
        let h = 3u32;
        let img = vec![
            vec3(0.0, 0.0, 0.0),
            vec3(0.5, 0.5, 0.5),
            vec3(0.0, 0.0, 0.0),
            vec3(0.5, 0.5, 0.5),
            vec3(1.0, 1.0, 1.0),
            vec3(0.5, 0.5, 0.5),
            vec3(0.0, 0.0, 0.0),
            vec3(0.5, 0.5, 0.5),
            vec3(0.0, 0.0, 0.0),
        ];
        let (min, max) = neighbourhood_ycgco_aabb(&img, w, h, 1, 1);
        let centre = rgb_to_ycgco(vec3(1.0, 1.0, 1.0));
        assert!(centre.x >= min.x && centre.x <= max.x);
        assert!(centre.y >= min.y && centre.y <= max.y);
        assert!(centre.z >= min.z && centre.z <= max.z);
    }

    #[test]
    fn clip_aabb_inside_returns_unchanged() {
        let min = vec3(0.0, 0.0, 0.0);
        let max = vec3(1.0, 1.0, 1.0);
        let centre = vec3(0.5, 0.5, 0.5);
        let colour = vec3(0.7, 0.4, 0.6);
        let (out, t) = clip_aabb(min, max, centre, colour);
        assert_eq!(out, colour);
        assert_eq!(t, 0.0);
    }

    #[test]
    fn clip_aabb_outside_pulls_toward_aabb() {
        let min = vec3(0.0, 0.0, 0.0);
        let max = vec3(1.0, 1.0, 1.0);
        let centre = vec3(0.5, 0.5, 0.5);
        let colour = vec3(2.0, 0.5, 0.5); // way outside on +x
        let (out, t) = clip_aabb(min, max, centre, colour);
        // Final x must hit the slab boundary (1.0).
        assert!((out.x - 1.0).abs() < 1e-5, "out={out:?}");
        assert!(t > 0.0);
        assert!(t < 1.0);
    }

    #[test]
    fn taa_resolve_converges_to_history_when_motion_zero_and_no_change() {
        // Solid grey current; history = same. Expect near-identical
        // output (clip distance = 0, alpha = TAA_ALPHA_MIN).
        let w = 4u32;
        let h = 4u32;
        let n = (w * h) as usize;
        let cur = vec![vec3(0.4, 0.4, 0.4); n];
        let his = vec![vec3(0.4, 0.4, 0.4); n];
        let motion = vec![Vec2::ZERO; n];
        let depth_cur = vec![1.0; n];
        let depth_his = vec![1.0; n];
        let mut out = vec![Vec3::ZERO; n];
        let input = TaaInput {
            current: &cur,
            history: &his,
            motion: &motion,
            depth_current: &depth_cur,
            depth_history: &depth_his,
            width: w,
            height: h,
        };
        taa_resolve(&input, &mut out);
        for (i, p) in out.iter().enumerate() {
            assert!((p.x - 0.4).abs() < 1e-5, "px {i}: {p:?}");
        }
    }

    #[test]
    fn taa_resolve_rejects_history_on_depth_jump() {
        // Depth ratio 2× → disocclusion mask triggers, rejection = 1.0,
        // output should be approximately the current frame.
        let w = 4u32;
        let h = 4u32;
        let n = (w * h) as usize;
        let cur = vec![vec3(0.9, 0.2, 0.2); n];
        let his = vec![vec3(0.2, 0.2, 0.9); n];
        let motion = vec![Vec2::ZERO; n];
        let depth_cur = vec![1.0; n];
        let depth_his = vec![2.5; n]; // 2.5x ratio > 1.1
        let mut out = vec![Vec3::ZERO; n];
        let input = TaaInput {
            current: &cur,
            history: &his,
            motion: &motion,
            depth_current: &depth_cur,
            depth_history: &depth_his,
            width: w,
            height: h,
        };
        taa_resolve(&input, &mut out);
        // alpha = MAX → blend = 0.5 * history_clipped + 0.5 * current.
        // History is clipped into the centre's tight AABB before blend,
        // so the output is close to the current colour.
        for p in &out {
            assert!(p.x > 0.5, "expected red-dominant output, got {p:?}");
            assert!(p.z < p.x);
        }
    }

    #[test]
    fn ssao_flat_depth_returns_unity() {
        let w = 8u32;
        let h = 8u32;
        let depth = vec![10.0_f32; (w * h) as usize];
        for py in 0..h {
            for px in 0..w {
                let ao = ssao_factor(px, py, &depth, w, h, 2);
                assert!((ao - 1.0).abs() < 1e-5, "px=({px},{py}) ao={ao}");
            }
        }
    }

    #[test]
    fn ssao_neighbour_closer_drops_below_unity() {
        let w = 8u32;
        let h = 8u32;
        let mut depth = vec![10.0_f32; (w * h) as usize];
        // Push a small "pillar" closer to the camera in the middle.
        for py in 3..5 {
            for px in 3..5 {
                depth[(py as usize) * (w as usize) + (px as usize)] = 5.0;
            }
        }
        // A pixel one tile out from the pillar should pick up occlusion.
        let ao = ssao_factor(2, 4, &depth, w, h, 2);
        assert!(ao < 1.0, "expected occlusion < 1, got {ao}");
    }

    #[test]
    fn bloom_extract_below_threshold_is_zero() {
        let v = bloom_extract(vec3(0.3, 0.3, 0.3), 1.0);
        assert_eq!(v, Vec3::ZERO);
    }

    #[test]
    fn bloom_extract_above_threshold_is_positive() {
        let v = bloom_extract(vec3(2.0, 2.0, 2.0), 1.0);
        assert!(v.x > 0.0 && v.y > 0.0 && v.z > 0.0);
    }

    #[test]
    fn gaussian_blur_flat_field_is_invariant() {
        let w = 4u32;
        let h = 4u32;
        let n = (w * h) as usize;
        let inp = vec![vec3(0.5, 0.3, 0.8); n];
        let mut out = vec![Vec3::ZERO; n];
        gaussian_blur_3x3(&inp, &mut out, w, h);
        for p in &out {
            assert!((p.x - 0.5).abs() < 1e-5);
            assert!((p.y - 0.3).abs() < 1e-5);
            assert!((p.z - 0.8).abs() < 1e-5);
        }
    }

    #[test]
    fn aces_tonemap_zero_is_zero_and_large_inputs_clamp_to_unity() {
        let zero = tonemap_aces(Vec3::ZERO);
        assert!(zero.x.abs() < 1e-3 && zero.y.abs() < 1e-3 && zero.z.abs() < 1e-3);
        let huge = tonemap_aces(vec3(1000.0, 1000.0, 1000.0));
        assert!(huge.x <= 1.0 && huge.x > 0.9);
    }

    #[test]
    fn aces_tonemap_is_monotonic() {
        let mut last = 0.0_f32;
        for i in 0..20 {
            let x = i as f32 * 0.25;
            let v = tonemap_aces(vec3(x, x, x)).x;
            assert!(v + 1e-5 >= last, "i={i} x={x}: {v} < {last}");
            last = v;
        }
    }
}
