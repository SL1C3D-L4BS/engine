//! Timing primitives for the cache observatory.
//!
//! Two paths:
//!
//! - [`Stopwatch`] — always available, wraps [`std::time::Instant`]. On Linux
//!   this routes to `clock_gettime(CLOCK_MONOTONIC_RAW)` and is reliable on
//!   every dev host.
//! - [`PerfCounters`] — optional, gated behind the `--with-perf-counters` CLI
//!   flag. Backed by [`crate::perf`] on Linux; a no-op on every other host.
//!   Falls back to wall-clock-only if `perf_event_open` is rejected.

use std::time::{Duration, Instant};

/// Always-on wall-clock timer.
pub struct Stopwatch {
    start: Instant,
}

impl Stopwatch {
    /// Starts a timer.
    #[inline]
    pub fn start() -> Self {
        Self {
            start: Instant::now(),
        }
    }

    /// Elapsed time since [`start`](Self::start).
    #[inline]
    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }
}

/// Aggregated cache-miss counts for one workload run. Every field is `None`
/// unless [`PerfCounters`] is open and the underlying event was accepted by
/// the kernel.
#[derive(Clone, Copy, Debug, Default)]
pub struct PerfSample {
    /// L1 data-cache read misses.
    pub l1d_misses: Option<u64>,
    /// L2 cache misses. Currently always `None` — L2 lacks a portable
    /// PERF_TYPE_HW_CACHE encoding and CPU-specific raw events are out of
    /// scope for the Phase 1 baseline.
    pub l2_misses: Option<u64>,
    /// Last-level cache read misses.
    pub llc_misses: Option<u64>,
    /// Retired CPU cycles.
    pub cycles: Option<u64>,
    /// Retired instructions.
    pub instructions: Option<u64>,
}

/// Handle to the kernel `perf_event_open` group. Owned plumbing; no
/// third-party perf-event crate (R-02). See [`crate::perf`] for the Linux
/// implementation.
pub struct PerfCounters {
    #[cfg(target_os = "linux")]
    inner: crate::perf::LinuxPerfCounters,
}

impl PerfCounters {
    /// Tries to open the counter group. Returns `Ok(None)` when the platform
    /// has no perf-event support (non-Linux builds) and `Err` when the
    /// caller asked for counters but the kernel refused.
    #[cfg(target_os = "linux")]
    pub fn try_open() -> Result<Option<Self>, std::io::Error> {
        match crate::perf::LinuxPerfCounters::try_open()? {
            Some(inner) => Ok(Some(Self { inner })),
            None => Ok(None),
        }
    }

    /// Tries to open the counter group. Always `Ok(None)` on non-Linux.
    #[cfg(not(target_os = "linux"))]
    pub fn try_open() -> Result<Option<Self>, std::io::Error> {
        Ok(None)
    }

    /// Resets all counters to zero and starts counting.
    pub fn start(&mut self) {
        #[cfg(target_os = "linux")]
        self.inner.start();
    }

    /// Snapshots and stops the counters.
    pub fn snapshot(&mut self) -> PerfSample {
        #[cfg(target_os = "linux")]
        return self.inner.snapshot();
        #[cfg(not(target_os = "linux"))]
        PerfSample::default()
    }
}
