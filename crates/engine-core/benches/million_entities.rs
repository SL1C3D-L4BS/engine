//! Million-entities frame bench (ADR-033 milestone gate).
//!
//! The Phase 3 portfolio target is **1 000 000 entities at 60 FPS on one
//! core**. This bench measures the wall-clock cost of one frame at the
//! milestone scale (and a few smaller scales for context). Numbers land
//! in `target/criterion/`; copy summary medians into
//! `docs/observatory/million-entities-baseline.md`.
//!
//! The bench harness is informational — not a CI gate. Runner noise
//! makes a hard threshold infeasible in shared CI; the milestone is
//! verified on a developer machine and captured in the observatory.
//!
//! The workload is intentionally simple: every entity carries
//! `Position` + `Velocity`, and one system per frame advances Position
//! by Velocity. Pure Table-archetype traversal across one column pair —
//! the hot path the archetype-SoA layout was designed for.

#![allow(missing_docs)]

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use engine_core::{Component, Entity, Phase, Schedule, World};
use engine_platform::ThreadPool;

#[derive(Component, Clone, Copy)]
struct Position {
    x: f32,
    y: f32,
    z: f32,
}

#[derive(Component, Clone, Copy)]
struct Velocity {
    dx: f32,
    dy: f32,
    dz: f32,
}

fn populate(world: &mut World, n: usize) {
    for i in 0..n as u32 {
        let e: Entity = world.spawn();
        let fi = i as f32;
        world.insert(
            e,
            Position {
                x: fi,
                y: fi * 0.5,
                z: -fi,
            },
        );
        world.insert(
            e,
            Velocity {
                dx: 0.01,
                dy: 0.02,
                dz: -0.01,
            },
        );
    }
}

fn build_schedule() -> Schedule {
    let mut schedule = Schedule::new();
    schedule.add_system_with_access(
        Phase::Update,
        "motion",
        &[Velocity::STABLE_ID],
        &[Position::STABLE_ID],
        |w: &mut World| {
            let mut updates: Vec<(Entity, f32, f32, f32)> = Vec::new();
            w.for_each::<Velocity>(|e, v| updates.push((e, v.dx, v.dy, v.dz)));
            for (e, dx, dy, dz) in updates {
                if let Some(p) = w.get_mut::<Position>(e) {
                    p.x += dx;
                    p.y += dy;
                    p.z += dz;
                }
            }
        },
    );
    schedule
}

fn frame(c: &mut Criterion) {
    let mut g = c.benchmark_group("million_entities/frame");
    // Sample size is small at 1M — one frame is ~ms-scale and we don't
    // want a 60 s bench wall-clock.
    g.sample_size(10);

    // Three scales: 10k (microbench-y, low variance), 100k (mid), 1M
    // (milestone). Each measures one frame end-to-end.
    for &n in &[10_000usize, 100_000, 1_000_000] {
        // Sequential path.
        g.bench_with_input(BenchmarkId::new("sequential", n), &n, |b, &n| {
            let mut world = World::new();
            populate(&mut world, n);
            let mut schedule = build_schedule();
            b.iter(|| {
                schedule.run(&mut world);
            });
        });
        // Parallel path. Same workload — one system, so the per-frame
        // benefit is dispatch overhead only; the test exists to track
        // regressions in `Schedule::run_on` on the hot path.
        g.bench_with_input(BenchmarkId::new("parallel", n), &n, |b, &n| {
            let pool = ThreadPool::with_default_workers();
            let mut world = World::new();
            populate(&mut world, n);
            let mut schedule = build_schedule();
            b.iter(|| {
                schedule.run_on(&mut world, &pool);
            });
        });
    }
    g.finish();
}

criterion_group!(benches, frame);
criterion_main!(benches);
