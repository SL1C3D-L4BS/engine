//! Monotonic time and frame pacing.
//!
//! Frame pacing is a top-tier architectural principle (spec IV.5, ADR-016):
//! frame-time *consistency* matters more than peak frame rate. [`FramePacer`]
//! implements the spec's sleep strategy — a coarse sleep for the bulk of the
//! idle time, then a busy-wait spin for the final fraction of a millisecond so
//! the frame boundary is hit precisely without burning a whole core.

use std::time::{Duration, Instant};

/// Returns a monotonic timestamp. Wraps [`Instant::now`], which is backed by
/// `CLOCK_MONOTONIC` on Linux.
#[inline]
pub fn now() -> Instant {
    Instant::now()
}

/// Paces a render loop to a fixed target frame rate.
#[derive(Clone, Copy, Debug)]
pub struct FramePacer {
    target: Duration,
    spin_margin: Duration,
}

impl FramePacer {
    /// Builds a pacer for `target_hz` frames per second.
    ///
    /// `target_hz` must be finite and positive.
    pub fn new(target_hz: f64) -> Self {
        assert!(
            target_hz.is_finite() && target_hz > 0.0,
            "frame rate must be finite and positive"
        );
        Self {
            target: Duration::from_secs_f64(1.0 / target_hz),
            spin_margin: Duration::from_micros(200),
        }
    }

    /// The target time budget for a single frame.
    #[inline]
    pub fn target(self) -> Duration {
        self.target
    }

    /// Blocks until `frame_start + target` has elapsed, then returns the total
    /// time the frame occupied.
    ///
    /// If the frame already overran its budget this returns immediately with
    /// the (over-budget) elapsed time — the caller can treat that as a stall.
    pub fn pace(self, frame_start: Instant) -> Duration {
        let deadline = frame_start + self.target;
        loop {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            let remaining = deadline - now;
            if remaining > self.spin_margin {
                // Coarse sleep for everything except the final spin margin.
                std::thread::sleep(remaining - self.spin_margin);
            } else {
                // Busy-wait the remainder for a precise frame boundary.
                std::hint::spin_loop();
            }
        }
        frame_start.elapsed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pacer_holds_the_frame_budget() {
        let pacer = FramePacer::new(120.0);
        let start = Instant::now();
        let elapsed = pacer.pace(start);
        // The frame must last at least the target, and pacing overhead must be
        // modest (well under a second of slack on an 8.33 ms budget).
        assert!(elapsed >= pacer.target());
        assert!(elapsed < pacer.target() + Duration::from_millis(5));
    }

    #[test]
    fn overrun_returns_immediately() {
        let pacer = FramePacer::new(1000.0);
        // A frame that "started" in the past is already over budget.
        let start = Instant::now() - Duration::from_millis(50);
        let elapsed = pacer.pace(start);
        assert!(elapsed >= Duration::from_millis(50));
    }
}
