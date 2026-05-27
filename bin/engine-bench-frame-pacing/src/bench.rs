//! Phase-5 frame-pacing scenario (ADR-053 §PR-5 + ADR-047 §1).
//!
//! Runs a deterministic per-frame workload:
//!
//! 1. Construct a synthetic HDR gradient at the *internal* render
//!    resolution. Pure CPU; same numbers every run.
//! 2. Bilinearly upscale to the *display* resolution using
//!    `engine_raster::bilinear_upscale` — the CPU reference for
//!    ADR-005's [`engine_render::OwnedBilinear`] placeholder.
//!
//! Per-frame wall-clock is captured via [`std::time::Instant`]; the
//! [`ScenarioReport`] aggregates summary statistics for the JSON
//! emitter.
//!
//! The PR-5 scenario does not (yet) drive the full deferred pipeline
//! through `engine-raster::sample::combined_deferred_scene` — that
//! integration lives in PR 6 when the self-hosted GPU runner stands up
//! per ADR-047 §2. The trait surface is what PR 5 exercises end-to-end.

use std::time::Instant;

use engine_math::Vec3;
use engine_raster::bilinear_upscale;
use engine_render::{OwnedBilinear, UpscaleCtx, UpscalerProvider, UpscalerRegistry};

/// Caller-supplied scenario knobs.
pub struct Scenario {
    /// Frames to measure.
    pub frames: u32,
    /// Internal render resolution `[w, h]`.
    pub input_extent: [u32; 2],
    /// Display resolution `[w, h]`.
    pub output_extent: [u32; 2],
}

/// Output of a scenario run.
pub struct ScenarioReport {
    /// Per-frame wall-clock in nanoseconds.
    pub frame_times_ns: Vec<u64>,
    /// Internal extent recorded in the report.
    pub input_extent: [u32; 2],
    /// Display extent recorded in the report.
    pub output_extent: [u32; 2],
    /// Mean frame time in milliseconds.
    pub mean_ms: f64,
    /// Min frame time in milliseconds.
    pub min_ms: f64,
    /// Max frame time in milliseconds.
    pub max_ms: f64,
}

/// Scenario errors.
#[derive(Debug)]
pub enum ScenarioError {
    /// Zero frames requested.
    NoFrames,
}

impl core::fmt::Display for ScenarioError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ScenarioError::NoFrames => f.write_str("scenario requires at least 1 frame"),
        }
    }
}

impl std::error::Error for ScenarioError {}

/// Run [`Scenario`]. Returns the [`ScenarioReport`] or a
/// [`ScenarioError`].
pub fn run_scenario(s: &Scenario) -> Result<ScenarioReport, ScenarioError> {
    if s.frames == 0 {
        return Err(ScenarioError::NoFrames);
    }
    let [in_w, in_h] = s.input_extent;
    let [out_w, out_h] = s.output_extent;
    let src = build_synthetic_hdr(in_w, in_h);
    let mut dst = vec![Vec3::ZERO; (out_w as usize) * (out_h as usize)];

    // Selection-logic smoke: the bilinear placeholder is the registered
    // owned fallback. We do not call `UpscalerRegistry::select` here
    // (the workspace `wgpu` dep is configured without backend features
    // for non-GPU CI; constructing a real `Device` would panic). The
    // PR-5 invariant — bilinear is the chosen path on every supported
    // host — is verified inside `engine-render`'s unit tests via the
    // registry's `kinds()` accessor.
    let _ = UpscalerRegistry::with_phase5_defaults();
    let provider = OwnedBilinear;

    let mut frame_times_ns: Vec<u64> = Vec::with_capacity(s.frames as usize);
    for frame in 0..s.frames {
        let t0 = Instant::now();
        // Per-frame work: full bilinear upscale of the synthetic HDR
        // gradient into `dst`. Deterministic — same inputs every frame
        // — but the wall-clock varies with CPU pressure, which is the
        // signal the bench captures.
        bilinear_upscale(&src, in_w, in_h, &mut dst, out_w, out_h);
        let mut scratch: u32 = 0;
        let jitter_vec = engine_raster::jitter_for_frame(frame as u64);
        let mut ctx = UpscaleCtx {
            frame_idx: frame as u64,
            jitter: [jitter_vec.x, jitter_vec.y],
            input_extent: s.input_extent,
            output_extent: s.output_extent,
            user: &mut scratch,
        };
        // Trait-surface dispatch: the placeholder returns a token; the
        // pixel work was the `bilinear_upscale` call above. This call
        // pins the trait shape across PR-5 (CPU) and PR-6+ (GPU) so a
        // regression in either pipeline shows up in the same hot path.
        let _ = provider
            .upscale(&mut ctx)
            .expect("OwnedBilinear must succeed");
        let elapsed = t0.elapsed().as_nanos() as u64;
        frame_times_ns.push(elapsed);
    }

    let (mean_ms, min_ms, max_ms) = summary_ms(&frame_times_ns);
    Ok(ScenarioReport {
        frame_times_ns,
        input_extent: s.input_extent,
        output_extent: s.output_extent,
        mean_ms,
        min_ms,
        max_ms,
    })
}

/// Synthetic HDR gradient: each pixel's RGB is a deterministic function
/// of `(x, y, w, h)`. Used as the bilinear upscaler's input. The exact
/// pattern is not load-bearing; only that it is non-constant and
/// reproducible across runs.
fn build_synthetic_hdr(w: u32, h: u32) -> Vec<Vec3> {
    let mut out = Vec::with_capacity((w as usize) * (h as usize));
    for y in 0..h {
        for x in 0..w {
            let fx = (x as f32) / (w.max(1) as f32);
            let fy = (y as f32) / (h.max(1) as f32);
            out.push(Vec3::new(fx, fy, 1.0 - fx * fy));
        }
    }
    out
}

fn summary_ms(times_ns: &[u64]) -> (f64, f64, f64) {
    if times_ns.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    let mut sum = 0.0_f64;
    for &t in times_ns {
        let ms = (t as f64) / 1_000_000.0;
        if ms < min {
            min = ms;
        }
        if ms > max {
            max = ms;
        }
        sum += ms;
    }
    let mean = sum / (times_ns.len() as f64);
    (mean, min, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_scenario_zero_frames_errors() {
        let s = Scenario {
            frames: 0,
            input_extent: [4, 4],
            output_extent: [8, 8],
        };
        let err = run_scenario(&s).err().expect("zero frames must error");
        assert!(matches!(err, ScenarioError::NoFrames));
    }

    #[test]
    fn run_scenario_smoke_small_resolution() {
        // Use a tiny resolution so the test is fast on every host.
        let s = Scenario {
            frames: 4,
            input_extent: [16, 8],
            output_extent: [32, 16],
        };
        let r = run_scenario(&s).expect("scenario must succeed");
        assert_eq!(r.frame_times_ns.len(), 4);
        assert_eq!(r.input_extent, [16, 8]);
        assert_eq!(r.output_extent, [32, 16]);
        assert!(r.max_ms >= r.min_ms);
        assert!(r.mean_ms >= 0.0);
        // Every frame should have a strictly positive wall-clock.
        for t in &r.frame_times_ns {
            assert!(*t > 0, "frame_time_ns must be > 0");
        }
    }

    #[test]
    fn build_synthetic_hdr_is_deterministic() {
        let a = build_synthetic_hdr(7, 5);
        let b = build_synthetic_hdr(7, 5);
        assert_eq!(a.len(), 35);
        assert_eq!(a, b);
    }

    #[test]
    fn summary_ms_matches_hand_computation() {
        let (mean, min, max) = summary_ms(&[1_000_000, 2_000_000, 3_000_000, 4_000_000]);
        assert!((mean - 2.5).abs() < 1e-9);
        assert!((min - 1.0).abs() < 1e-9);
        assert!((max - 4.0).abs() < 1e-9);
    }
}
