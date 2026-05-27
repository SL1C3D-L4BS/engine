//! Summary statistics over frame-time samples (ADR-047 §3).
//!
//! Owned implementations — same discipline as the rest of the bench
//! binary. Both helpers take `&[u64]` of per-frame durations in
//! nanoseconds and return milliseconds, matching the JSON report's
//! `*_ms` shape so callers do not have to rescale.

/// Population standard deviation of `times_ns`, in milliseconds.
///
/// Population (divide by `N`) rather than sample (divide by `N - 1`) —
/// the bench captures every frame in the scenario, not a random sample
/// of an underlying distribution. Returns `0.0` for an empty slice.
pub fn stddev_ms(times_ns: &[u64]) -> f64 {
    if times_ns.is_empty() {
        return 0.0;
    }
    let n = times_ns.len() as f64;
    let mean_ns = times_ns.iter().map(|&t| t as f64).sum::<f64>() / n;
    let var_ns2 = times_ns
        .iter()
        .map(|&t| {
            let d = (t as f64) - mean_ns;
            d * d
        })
        .sum::<f64>()
        / n;
    var_ns2.sqrt() / 1_000_000.0
}

/// 99th percentile of `times_ns`, in milliseconds, via nearest-rank.
///
/// `idx = ceil(0.99 * n) - 1`, clamped to `[0, n - 1]`. For `n < 100`
/// this collapses to "the highest sample" — consistent with how the
/// ADR-047 budget is interpreted on short scenarios. Returns `0.0` for
/// an empty slice.
pub fn p99_ms(times_ns: &[u64]) -> f64 {
    if times_ns.is_empty() {
        return 0.0;
    }
    let mut sorted: Vec<u64> = times_ns.to_vec();
    sorted.sort_unstable();
    let n = sorted.len();
    let idx = ((0.99_f64 * n as f64).ceil() as usize)
        .saturating_sub(1)
        .min(n - 1);
    (sorted[idx] as f64) / 1_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stddev_population_matches_hand_computation() {
        // [1, 2, 3, 4] ms; mean = 2.5; deviations squared = [2.25, 0.25, 0.25, 2.25];
        // population variance = 5/4 = 1.25; stddev = sqrt(1.25) ≈ 1.118034.
        let s = stddev_ms(&[1_000_000, 2_000_000, 3_000_000, 4_000_000]);
        assert!((s - 1.118).abs() < 0.01, "stddev was {s}");
    }

    #[test]
    fn stddev_empty_is_zero() {
        assert_eq!(stddev_ms(&[]), 0.0);
    }

    #[test]
    fn stddev_single_sample_is_zero() {
        assert_eq!(stddev_ms(&[5_000_000]), 0.0);
    }

    #[test]
    fn p99_short_run_picks_max_sample() {
        // n = 4: ceil(0.99 * 4) - 1 = 4 - 1 = 3 → sorted[3] = max
        let p = p99_ms(&[4_000_000, 1_000_000, 3_000_000, 2_000_000]);
        assert!((p - 4.0).abs() < 1e-9, "p99 was {p}");
    }

    #[test]
    fn p99_empty_is_zero() {
        assert_eq!(p99_ms(&[]), 0.0);
    }

    #[test]
    fn p99_at_n_100_is_99th_index() {
        // n = 100: ceil(99.0) - 1 = 99 - 1 = 98 → sorted[98]
        // Values 0..100 ms; sorted[98] = 98 ms.
        let mut samples: Vec<u64> = (0..100).map(|i| i * 1_000_000).collect();
        samples.reverse();
        let p = p99_ms(&samples);
        assert!((p - 98.0).abs() < 1e-9, "p99 was {p}");
    }
}
