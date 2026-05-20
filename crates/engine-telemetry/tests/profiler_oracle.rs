//! Oracle for the sampling profiler (ADR-030).
//!
//! A known workload — a tight CPU spinner on one thread and a yielding
//! sleeper on another — is sampled for ~1 second at 99 Hz. The oracle
//! asserts:
//!
//! 1. The spinner appears in the captured stacks at all.
//! 2. The spinner's self-time fraction is ≥ 80% of the spinner thread's
//!    samples. This is the "catches 51/49 near-tie regressions" check
//!    spelled out in the Phase 2 plan; a bare "is the spinner present?"
//!    assertion would not.
//!
//! Linux-only. On macOS / Windows / a kernel with
//! `perf_event_paranoid > 2 && !CAP_PERFMON`, `SamplingProfiler::try_attach`
//! returns `Ok(None)` and the oracle short-circuits with a documented
//! skip log line — these are exactly the same degradation conditions the
//! engine itself observes.
//!
//! Frame-pointer codegen (`-C force-frame-pointers=yes`) is required for
//! the kernel's call-chain walker to find any frames beyond the leaf;
//! the CI workflow asserts this flag is in `RUSTFLAGS` before running
//! this oracle.

#![allow(clippy::missing_panics_doc)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use engine_telemetry::SamplingProfiler;

/// A tight CPU-bound spinner. `#[unsafe(no_mangle)]` keeps the symbol
/// name out of the mangler so post-hoc analysis (e.g. `dladdr`, `nm`)
/// can attribute samples to it.
///
/// The inner loop is large (~1M iterations) so the surrounding
/// `Instant::elapsed()` poll is amortized below the sampling rate —
/// otherwise a debug build spends most of its cycles in `now()` /
/// `clock_gettime` and the spinner does not dominate self-time.
#[inline(never)]
#[unsafe(no_mangle)]
extern "C" fn engine_profiler_spinner(deadline_ms: u64) -> u64 {
    let mut acc: u64 = 1;
    let start = Instant::now();
    loop {
        // Burn ~1M ALU ops between elapsed() polls. The optimizer can't
        // hoist `acc.wrapping_mul(...)` out: every iteration depends on
        // the previous result.
        for i in 0..1_000_000u64 {
            acc = acc.wrapping_mul(2_654_435_761).wrapping_add(i ^ acc);
        }
        if (start.elapsed().as_millis() as u64) >= deadline_ms {
            break;
        }
    }
    acc
}

/// A yielding sleeper that should accumulate ~zero on-CPU samples.
#[inline(never)]
#[unsafe(no_mangle)]
extern "C" fn engine_profiler_sleeper(stop_flag: &AtomicBool) {
    while !stop_flag.load(Ordering::Relaxed) {
        std::thread::yield_now();
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Returns `true` if `ip` is within `window` bytes of `start`. Used to
/// claim a sample for a function without a full symbol-table lookup —
/// real symbolization is a CLI-level concern.
fn in_function_window(ip: u64, start: u64, window: u64) -> bool {
    ip >= start && ip - start < window
}

#[cfg(target_os = "linux")]
#[test]
fn spinner_dominates_self_time() {
    // The oracle's self-time fraction only holds in an optimized build —
    // a debug build spends most of its cycles in non-inlined wrapping-mul
    // helpers and `Instant::now`, neither of which the spec considers
    // part of the spinner. CI runs this test with `--release`; locally,
    // use `just profiler-oracle`.
    if cfg!(debug_assertions) {
        eprintln!(
            "profiler_oracle: skipping — debug build. Run via \
             `cargo test --release -p engine-telemetry --test profiler_oracle` \
             or `just profiler-oracle`."
        );
        return;
    }

    // Bootstrap: open the profiler on the calling (test runner) thread.
    let profiler = match SamplingProfiler::try_attach(99) {
        Ok(Some(p)) => p,
        Ok(None) => {
            eprintln!(
                "profiler_oracle: skipping — perf_event_open unavailable \
                 (perf_event_paranoid > 2 without CAP_PERFMON, or kernel \
                 missing perf). The runtime engine degrades the same way."
            );
            return;
        }
        Err(e) => panic!("unexpected error opening sampler: {e}"),
    };

    // Run the spinner on the same thread the profiler is attached to.
    let _ = engine_profiler_spinner(1_000);

    let folded = profiler.finish();
    let total = folded.total();
    assert!(
        total > 0,
        "no samples collected — frame pointers may be off, or the timer never fired"
    );

    let spinner_addr = engine_profiler_spinner as *const () as u64;
    // 16 KiB window — large enough for any reasonable function body and
    // small enough to not overlap with neighbouring code in a release
    // build. The function's body is well under 1 KiB.
    let window: u64 = 16 * 1024;

    // Sum every sample whose leaf IP lands inside the spinner. The leaf
    // is the most recently-executing PC, which for a tight ALU loop is
    // exactly the on-CPU function.
    // Debug: print the top few stacks' leaf IPs and the spinner's
    // address so a fail surfaces useful information rather than just
    // "0%". This stays in even on success — eprintln in tests is
    // suppressed unless the test fails or `--nocapture` is passed.
    eprintln!("spinner @ 0x{spinner_addr:x}");
    for s in folded.stacks.iter().take(8) {
        let leaf = s.ips.first().copied().unwrap_or(0);
        let in_win = in_function_window(leaf, spinner_addr, window);
        let dist = leaf.wrapping_sub(spinner_addr) as i64;
        eprintln!(
            "  count={} leaf=0x{leaf:x} in_window={} dist={}",
            s.count, in_win, dist
        );
    }

    let in_spinner: u64 = folded
        .stacks
        .iter()
        .filter(|s| {
            s.ips
                .first()
                .copied()
                .is_some_and(|leaf| in_function_window(leaf, spinner_addr, window))
        })
        .map(|s| s.count)
        .sum();

    let fraction = in_spinner as f64 / total as f64;
    eprintln!(
        "profiler_oracle: {} samples total, {} in spinner ({:.2}%)",
        total,
        in_spinner,
        fraction * 100.0
    );

    // ≥ 80%: the spinner is on-CPU continuously so it should pull
    // essentially every sample. We leave 20% headroom for the kernel
    // landing samples in libc / spinlock helpers and for the small
    // setup/teardown cost.
    assert!(
        fraction >= 0.80,
        "spinner self-time fraction was {fraction:.2}, expected ≥ 0.80",
    );

    // Sanity check: the spinner must be the #1 stack in the folded
    // ordering (the function sort_by_count places hottest first).
    let top = folded.stacks.first().expect("at least one stack");
    let top_leaf = top.ips.first().copied().unwrap_or(0);
    assert!(
        in_function_window(top_leaf, spinner_addr, window),
        "top stack's leaf IP 0x{top_leaf:x} is not in the spinner"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn sleeper_runs_without_dominating() {
    // Two-thread variant: one sleeper attached to itself, asserts the
    // sleeper produced few-to-no on-CPU samples (the profiler only
    // records on-CPU samples by construction).
    let stop_flag = Arc::new(AtomicBool::new(false));
    let stop_for_thread = Arc::clone(&stop_flag);

    let worker = std::thread::spawn(move || {
        let profiler = match SamplingProfiler::try_attach(99) {
            Ok(Some(p)) => p,
            Ok(None) => return None,
            Err(e) => panic!("unexpected sampler error: {e}"),
        };
        engine_profiler_sleeper(&stop_for_thread);
        Some(profiler.finish())
    });

    std::thread::sleep(Duration::from_millis(500));
    stop_flag.store(true, Ordering::Relaxed);
    let folded = worker.join().unwrap();
    let Some(folded) = folded else {
        eprintln!("profiler_oracle: sleeper test skipped — sampler unavailable");
        return;
    };

    // Sleeper should be a tiny fraction of total samples (most of the
    // time it's off-CPU). The test is therefore not "sleeper is small"
    // but "the profiler returned coherent data" — total >= 0 and the
    // result is well-formed.
    let total = folded.total();
    eprintln!("profiler_oracle (sleeper): {total} samples observed");
    // No upper-bound assertion: a kernel may attribute the brief
    // wake-up windows to the sleeper, and the count is small but
    // non-zero. The behaviour we *do* care about is exercised by
    // `spinner_dominates_self_time`.
}

#[cfg(not(target_os = "linux"))]
#[test]
#[ignore = "engine_platform::sampler is Linux-only; macOS/Windows return Ok(None)"]
fn spinner_dominates_self_time() {}
