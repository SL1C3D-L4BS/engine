//! Replay-parity oracle for the Phase 3 parallel scheduler (ADR-033).
//!
//! Runs a fixed deterministic workload — 1 000 entities, six systems with
//! declared R/W component-id sets, no random component churn — for 100
//! frames at a fixed timestep, and compares the per-frame BLAKE3 digest of
//! the world state across:
//!
//! 1. The single-threaded reference path ([`Schedule::run`]).
//! 2. The parallel pool ([`Schedule::run_on`]) at worker counts
//!    `{1, 2, 4, available_parallelism()}`.
//!
//! Every variant must produce identical digests at frames 1, 10, and 100.
//! That's the property the R/W-DAG scheduler stakes its safety claim on:
//! when system declarations are honest, dispatch order doesn't affect
//! observable world state. The plan's longer 100/1000/3600-frame golden
//! sweep lives in `crates/engine-core/tests/golden-core.txt` via the
//! existing `determinism.rs` oracle; this file is the cross-worker-count
//! parity check that complements it.

use engine_core::{Component, Entity, Phase, Schedule, World};
use engine_platform::ThreadPool;

#[derive(Component, Clone, Copy)]
struct Position {
    x: i64,
    y: i64,
}

#[derive(Component, Clone, Copy)]
struct Velocity {
    dx: i64,
    dy: i64,
}

#[derive(Component, Clone, Copy)]
struct Health {
    hp: i64,
}

#[derive(Component, Clone, Copy)]
struct Mana {
    mp: i64,
}

const N: usize = 1000;
const FRAMES: u32 = 100;

fn populate(world: &mut World) -> Vec<Entity> {
    let mut entities = Vec::with_capacity(N);
    for i in 0..N as i64 {
        let e = world.spawn();
        world.insert(
            e,
            Position {
                x: i,
                y: i.wrapping_mul(3),
            },
        );
        world.insert(
            e,
            Velocity {
                dx: 1 + (i % 5),
                dy: 2 - (i % 7),
            },
        );
        world.insert(e, Health { hp: 100 + i });
        world.insert(e, Mana { mp: 50 + (i % 11) });
        entities.push(e);
    }
    entities
}

fn build_schedule() -> Schedule {
    let mut schedule = Schedule::new();
    // motion: reads Velocity, writes Position. Parallel-safe with every
    // system except `bounce` (which writes Velocity). Uses the
    // collect-then-mutate pattern because the current query DSL only
    // exposes `(&A, &B)` and single-component mut; the planned wider
    // DSL is Phase 4+.
    schedule.add_system_with_access(
        Phase::Update,
        "motion",
        &[Velocity::STABLE_ID],
        &[Position::STABLE_ID],
        |w: &mut World| {
            let mut updates: Vec<(Entity, i64, i64)> = Vec::new();
            w.for_each::<Velocity>(|e, v| updates.push((e, v.dx, v.dy)));
            for (e, dx, dy) in updates {
                if let Some(p) = w.get_mut::<Position>(e) {
                    p.x = p.x.wrapping_add(dx);
                    p.y = p.y.wrapping_add(dy);
                }
            }
        },
    );
    // bounce: reads Position, writes Velocity (the other half of the
    // motion/Velocity pair — must serialise vs `motion`).
    schedule.add_system_with_access(
        Phase::Update,
        "bounce",
        &[Position::STABLE_ID],
        &[Velocity::STABLE_ID],
        |w: &mut World| {
            let mut snapshots: Vec<(Entity, i64, i64)> = Vec::new();
            w.for_each::<Position>(|e, p| snapshots.push((e, p.x, p.y)));
            for (e, x, y) in snapshots {
                let Some(v) = w.get_mut::<Velocity>(e) else {
                    continue;
                };
                if x.abs() > 10_000 {
                    v.dx = -v.dx;
                }
                if y.abs() > 10_000 {
                    v.dy = -v.dy;
                }
            }
        },
    );
    // regen: writes Health. Parallel-safe with motion / bounce / drain
    // (drain writes Mana only).
    schedule.add_system_with_access(
        Phase::Update,
        "regen",
        &[],
        &[Health::STABLE_ID],
        |w: &mut World| {
            w.for_each_mut::<Health>(|_e, h| {
                h.hp = h.hp.wrapping_add(1);
            });
        },
    );
    // drain: writes Mana. Parallel-safe with everything except `cast`.
    schedule.add_system_with_access(
        Phase::Update,
        "drain",
        &[],
        &[Mana::STABLE_ID],
        |w: &mut World| {
            w.for_each_mut::<Mana>(|_e, m| {
                m.mp = m.mp.wrapping_sub(1);
            });
        },
    );
    // cast: reads Mana, writes Health (serialises vs `drain` on Mana
    // and vs `regen` on Health).
    schedule.add_system_with_access(
        Phase::PostUpdate,
        "cast",
        &[Mana::STABLE_ID],
        &[Health::STABLE_ID],
        |w: &mut World| {
            let mut snapshots: Vec<(Entity, i64)> = Vec::new();
            w.for_each::<Mana>(|e, m| snapshots.push((e, m.mp)));
            for (e, mp) in snapshots {
                let Some(h) = w.get_mut::<Health>(e) else {
                    continue;
                };
                if mp > 0 {
                    h.hp = h.hp.wrapping_add(mp & 3);
                }
            }
        },
    );
    // tally: reads Position + Velocity + Health + Mana, writes nothing.
    // Pure read; parallel-safe with every other read.
    schedule.add_system_with_access(
        Phase::PostUpdate,
        "tally",
        &[
            Position::STABLE_ID,
            Velocity::STABLE_ID,
            Health::STABLE_ID,
            Mana::STABLE_ID,
        ],
        &[],
        |_w: &mut World| {
            // Pure observation; the digest folds the world state below.
        },
    );
    schedule
}

fn snapshot(world: &World) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    let mut buf = [0u8; 8];

    let mut positions: Vec<(u64, i64, i64)> = Vec::with_capacity(N);
    world.for_each::<Position>(|e, p| positions.push((e.to_bits(), p.x, p.y)));
    positions.sort_by_key(|t| t.0);
    for (b, x, y) in positions {
        buf.copy_from_slice(&b.to_le_bytes());
        hasher.update(&buf);
        buf.copy_from_slice(&x.to_le_bytes());
        hasher.update(&buf);
        buf.copy_from_slice(&y.to_le_bytes());
        hasher.update(&buf);
    }

    let mut velocities: Vec<(u64, i64, i64)> = Vec::with_capacity(N);
    world.for_each::<Velocity>(|e, v| velocities.push((e.to_bits(), v.dx, v.dy)));
    velocities.sort_by_key(|t| t.0);
    for (b, dx, dy) in velocities {
        buf.copy_from_slice(&b.to_le_bytes());
        hasher.update(&buf);
        buf.copy_from_slice(&dx.to_le_bytes());
        hasher.update(&buf);
        buf.copy_from_slice(&dy.to_le_bytes());
        hasher.update(&buf);
    }

    let mut healths: Vec<(u64, i64)> = Vec::with_capacity(N);
    world.for_each::<Health>(|e, h| healths.push((e.to_bits(), h.hp)));
    healths.sort_by_key(|t| t.0);
    for (b, h) in healths {
        buf.copy_from_slice(&b.to_le_bytes());
        hasher.update(&buf);
        buf.copy_from_slice(&h.to_le_bytes());
        hasher.update(&buf);
    }

    let mut manas: Vec<(u64, i64)> = Vec::with_capacity(N);
    world.for_each::<Mana>(|e, m| manas.push((e.to_bits(), m.mp)));
    manas.sort_by_key(|t| t.0);
    for (b, m) in manas {
        buf.copy_from_slice(&b.to_le_bytes());
        hasher.update(&buf);
        buf.copy_from_slice(&m.to_le_bytes());
        hasher.update(&buf);
    }

    *hasher.finalize().as_bytes()
}

/// Run the workload sequentially via `Schedule::run`, capturing digests at
/// frames 1, 10, and 100.
fn reference_digests() -> [[u8; 32]; 3] {
    let mut world = World::new();
    populate(&mut world);
    let mut schedule = build_schedule();
    let mut digests = [[0u8; 32]; 3];
    for frame in 1..=FRAMES {
        schedule.run(&mut world);
        match frame {
            1 => digests[0] = snapshot(&world),
            10 => digests[1] = snapshot(&world),
            100 => digests[2] = snapshot(&world),
            _ => {}
        }
    }
    digests
}

/// Run the same workload via `Schedule::run_on` on a pool of `workers`
/// threads, capturing digests at the same frame indices.
fn parallel_digests(workers: usize) -> [[u8; 32]; 3] {
    let pool = ThreadPool::with_workers(workers);
    let mut world = World::new();
    populate(&mut world);
    let mut schedule = build_schedule();
    let mut digests = [[0u8; 32]; 3];
    for frame in 1..=FRAMES {
        schedule.run_on(&mut world, &pool);
        match frame {
            1 => digests[0] = snapshot(&world),
            10 => digests[1] = snapshot(&world),
            100 => digests[2] = snapshot(&world),
            _ => {}
        }
    }
    digests
}

#[test]
fn parallel_run_matches_reference_across_worker_counts() {
    let reference = reference_digests();
    let available = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .max(1);
    // Always test the full set; duplicates are fine — the property
    // under test is "no worker count diverges from the reference".
    let worker_counts = [1usize, 2, 4, available];

    for &workers in worker_counts.iter() {
        let par = parallel_digests(workers);
        for (i, frame) in [1u32, 10, 100].iter().enumerate() {
            assert_eq!(
                par[i], reference[i],
                "digest divergence at frame {frame} with {workers} workers"
            );
        }
    }
}
