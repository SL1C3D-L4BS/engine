//! Phase-5 frame-pacing scenario (ADR-053 §PR-5 + ADR-047 §1).
//!
//! The bench runs a deterministic per-frame workload and captures
//! per-frame wall-clock. The roll-up — p99, σ, mean, min, max — is
//! the headline metric ADR-016 + ADR-047 §3 calibrate against.
//!
//! ## Workload (PR-6: full CPU oracle + bilinear upscale)
//!
//! Per frame:
//!
//! 1. Drive the deferred CPU oracle —
//!    `engine_raster::sample::combined_deferred_scene()` produces a
//!    128×72 framebuffer running the full Track-A path (CSM cascades
//!    rendered into an atlas, cluster-light binning, per-pixel
//!    Cook-Torrance accumulation). This is the CPU-side ground truth
//!    the GPU runner will pixel-parity against under ADR-046.
//! 2. Bilinearly upscale the framebuffer (decoded from sRGB to linear
//!    [`Vec3`]) into the bench's `output_extent` — the
//!    [`engine_render::OwnedBilinear`] placeholder oracle the trait
//!    surface ships with.
//!
//! Per-frame wall-clock is `std::time::Instant`-based: the bench wraps
//! the entire (deferred + upscale) sequence in a single timer. Per-pass
//! timing would mislead on the CPU oracle — the GPU-runner pass cost
//! distribution differs from the CPU's by orders of magnitude.
//!
//! PR-5 shipped the synthetic-HDR-gradient stand-in. PR-6 swaps it for
//! the full pipeline. The synthetic path remains in the repo's history
//! for reference; the bench API is unchanged.

use std::time::Instant;

use engine_math::Vec3;
use engine_raster::{bilinear_upscale, combined_deferred_scene};
use engine_render::{OwnedBilinear, UpscaleCtx, UpscalerProvider, UpscalerRegistry};

/// Caller-supplied scenario knobs.
pub struct Scenario {
    /// Frames to measure.
    pub frames: u32,
    /// Internal render resolution `[w, h]`. The CPU oracle is fixed at
    /// 128×72; `input_extent` is recorded in the report and is the GPU
    /// runner's authoritative render resolution.
    pub input_extent: [u32; 2],
    /// Display resolution `[w, h]` — the upscaler's output extent.
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
    let [out_w, out_h] = s.output_extent;
    let mut upscale_dst = vec![Vec3::ZERO; (out_w as usize) * (out_h as usize)];

    // PR-5 selection-logic smoke: the bilinear placeholder is the
    // registered owned fallback. The bench cannot call
    // `UpscalerRegistry::select` here because the workspace `wgpu` dep
    // is built without backend features (no `engine_gpu::Device`). The
    // PR-5 invariant — bilinear is the chosen path on every supported
    // host — is verified inside `engine-render`'s unit tests; the bench
    // assumes the placeholder.
    let _ = UpscalerRegistry::with_phase6_defaults();
    let provider = OwnedBilinear;

    let mut frame_times_ns: Vec<u64> = Vec::with_capacity(s.frames as usize);
    for frame in 0..s.frames {
        let t0 = Instant::now();
        // Step 1: drive the deferred CPU oracle. Each frame builds the
        // cascades + cluster grid + lighting accumulation from scratch
        // — the workload is deterministic but not cached.
        let scene = combined_deferred_scene();
        let in_w = scene.framebuffer.width();
        let in_h = scene.framebuffer.height();
        // Decode sRGB-encoded framebuffer bytes into linear [`Vec3`]
        // for the upscaler. The bench's upscale stage operates in
        // linear space, matching ADR-005's specification.
        let upscale_src: Vec<Vec3> = scene
            .framebuffer
            .color()
            .iter()
            .map(|p| {
                let (r, g, b, _) = p.to_linear();
                Vec3::new(r, g, b)
            })
            .collect();
        // Step 2: bilinearly upscale to the display extent.
        bilinear_upscale(&upscale_src, in_w, in_h, &mut upscale_dst, out_w, out_h);

        // Trait-surface dispatch: the placeholder returns a token; the
        // pixel work was the `bilinear_upscale` call above. This call
        // pins the trait shape across the CPU oracle (PR 5+6) and the
        // GPU path (PR 6+) so a regression in either pipeline shows up
        // in the same hot path.
        let mut scratch: u32 = 0;
        let jitter_vec = engine_raster::jitter_for_frame(frame as u64);
        let mut ctx = UpscaleCtx {
            frame_idx: frame as u64,
            jitter: [jitter_vec.x, jitter_vec.y],
            input_extent: [in_w, in_h],
            output_extent: s.output_extent,
            quality: engine_render::Quality::default(),
            user: &mut scratch,
        };
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
        // The deferred oracle is fixed at 128×72; we just upscale to a
        // small target to keep the test fast on every host.
        let s = Scenario {
            frames: 2,
            input_extent: [128, 72],
            output_extent: [256, 144],
        };
        let r = run_scenario(&s).expect("scenario must succeed");
        assert_eq!(r.frame_times_ns.len(), 2);
        assert_eq!(r.input_extent, [128, 72]);
        assert_eq!(r.output_extent, [256, 144]);
        assert!(r.max_ms >= r.min_ms);
        assert!(r.mean_ms >= 0.0);
        // Every frame should have a strictly positive wall-clock.
        for t in &r.frame_times_ns {
            assert!(*t > 0, "frame_time_ns must be > 0");
        }
    }

    #[test]
    fn summary_ms_matches_hand_computation() {
        let (mean, min, max) = summary_ms(&[1_000_000, 2_000_000, 3_000_000, 4_000_000]);
        assert!((mean - 2.5).abs() < 1e-9);
        assert!((min - 1.0).abs() < 1e-9);
        assert!((max - 4.0).abs() < 1e-9);
    }
}
