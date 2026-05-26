//! TAA jitter determinism oracle (ADR-042 §Verification).
//!
//! `jitter_for_frame(0..N)` must be byte-identical across runs and
//! across architectures — the engine-math arithmetic guarantee
//! (ADR-023). The shadow cascade pass (ADR-040 §5) cross-references
//! the same sequence, so a drift in this sequence is a CSM regression.

use engine_raster::post_fx::{TAA_JITTER_PERIOD, jitter_for_frame};

/// Sequence captured from the reference implementation. Drifting from
/// these values means either the Halton implementation changed or the
/// frame→index map changed; both are ADR violations.
///
/// Values are the un-shifted Halton samples (subtract 0.5 to convert
/// to the sub-pixel offset).
const REFERENCE_HALTON: [(f32, f32); 8] = [
    (0.5, 1.0 / 3.0),
    (0.25, 2.0 / 3.0),
    (0.75, 1.0 / 9.0),
    (0.125, 4.0 / 9.0),
    (0.625, 7.0 / 9.0),
    (0.375, 2.0 / 9.0),
    (0.875, 5.0 / 9.0),
    (0.0625, 8.0 / 9.0),
];

#[test]
fn first_eight_frames_match_reference_sequence() {
    for (frame, &(hx, hy)) in REFERENCE_HALTON.iter().enumerate() {
        let j = jitter_for_frame(frame as u64);
        let ex = hx - 0.5;
        let ey = hy - 0.5;
        assert!(
            (j.x - ex).abs() < 1e-6,
            "frame {frame} x: got {} want {ex}",
            j.x
        );
        assert!(
            (j.y - ey).abs() < 1e-6,
            "frame {frame} y: got {} want {ey}",
            j.y
        );
    }
}

#[test]
fn period_eight_repeats_exactly() {
    for frame in 0..1024u64 {
        let a = jitter_for_frame(frame);
        let b = jitter_for_frame(frame + TAA_JITTER_PERIOD as u64);
        assert!((a.x - b.x).abs() < 1e-6);
        assert!((a.y - b.y).abs() < 1e-6);
    }
}

#[test]
fn jitter_stays_within_pixel() {
    // Sub-pixel jitter must never exceed ±0.5 (the half-pixel
    // contract upscalers consume).
    for frame in 0..1024u64 {
        let j = jitter_for_frame(frame);
        assert!(j.x.abs() <= 0.5);
        assert!(j.y.abs() <= 0.5);
    }
}
