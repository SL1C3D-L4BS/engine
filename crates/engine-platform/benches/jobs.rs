//! Criterion bench for the Phase 3 job system (ADR-032).
//!
//! Three workload shapes are exercised:
//!
//! 1. **linear** — N jobs forming a single dependency chain. No parallelism
//!    available; measures the per-job dispatch + handoff overhead.
//! 2. **fan_out** — N jobs, no edges. Perfect parallelism upper bound.
//! 3. **mixed_grain** — Half the jobs do "short" work, the other half
//!    "long" work. No edges; measures steal latency under heterogeneous
//!    grain.
//!
//! Bench numbers land in `target/criterion/`; copy summary numbers into
//! `docs/observatory/jobs-baseline.md`.

#![allow(missing_docs)]

use criterion::{Criterion, criterion_group, criterion_main};
use engine_platform::{JobGraph, ThreadPool};

fn linear_chain(c: &mut Criterion) {
    let pool = ThreadPool::with_default_workers();
    let mut g = c.benchmark_group("jobs/linear");
    for &n in &[32usize, 128, 512] {
        g.bench_function(format!("n={n}"), |b| {
            b.iter(|| {
                let mut graph = JobGraph::new();
                for i in 0..n {
                    // Share one write key across every job → forces a
                    // dependency chain through the R/W conflict rule.
                    graph.add_job(&[], &[0u64], move || {
                        std::hint::black_box(i);
                    });
                }
                graph.run_on(&pool);
            });
        });
    }
    g.finish();
}

fn fan_out(c: &mut Criterion) {
    let pool = ThreadPool::with_default_workers();
    let mut g = c.benchmark_group("jobs/fan_out");
    for &n in &[64usize, 256, 1024] {
        g.bench_function(format!("n={n}"), |b| {
            b.iter(|| {
                let mut graph = JobGraph::new();
                for i in 0..n {
                    // Each job touches a disjoint slot → no edges.
                    let slot = i as u64;
                    graph.add_job(&[], &[slot], move || {
                        std::hint::black_box(slot);
                    });
                }
                graph.run_on(&pool);
            });
        });
    }
    g.finish();
}

fn mixed_grain(c: &mut Criterion) {
    let pool = ThreadPool::with_default_workers();
    let mut g = c.benchmark_group("jobs/mixed_grain");
    g.sample_size(20);
    for &n in &[64usize, 256] {
        g.bench_function(format!("n={n}"), |b| {
            b.iter(|| {
                let mut graph = JobGraph::new();
                for i in 0..n {
                    let slot = i as u64;
                    let heavy = i % 2 == 0;
                    graph.add_job(&[], &[slot], move || {
                        let mut acc = slot;
                        let iters = if heavy { 2_000 } else { 32 };
                        for k in 0..iters {
                            acc = acc.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(k);
                        }
                        std::hint::black_box(acc);
                    });
                }
                graph.run_on(&pool);
            });
        });
    }
    g.finish();
}

criterion_group!(benches, linear_chain, fan_out, mixed_grain);
criterion_main!(benches);
