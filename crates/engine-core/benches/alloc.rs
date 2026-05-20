//! Criterion benches for the engine-core arena allocators.
//!
//! Run with `just bench` (or `cargo bench -p engine-core --bench alloc`).
//! These benches are intentionally not wired into the `just ci` gate — bench
//! numbers are too runner-noisy to fail a CI build on. Commit baseline
//! numbers to `docs/observatory/arena-baseline.md` after every meaningful
//! arena change (ADR-026).

// Criterion's `criterion_group!` macro emits a `pub fn` without a doc
// comment; suppress the workspace-wide `missing_docs = "deny"` for this
// single bench file. The benches are not part of the public API.
#![allow(missing_docs)]

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use engine_core::alloc::{Arena, GeneralArena, LinearArena, PoolArena, RingArena};

fn linear_bump_64b(c: &mut Criterion) {
    c.bench_function("linear_bump_64b", |b| {
        b.iter(|| {
            let mut arena = LinearArena::with_capacity(1 << 20);
            for _ in 0..1024 {
                let slot = arena.alloc(64, 8).unwrap();
                black_box(slot.as_mut_ptr());
            }
        });
    });
}

fn linear_bump_mixed_alignment(c: &mut Criterion) {
    let sizes_aligns: [(usize, usize); 6] = [
        (16, 8),
        (64, 16),
        (128, 32),
        (256, 64),
        (512, 16),
        (1024, 8),
    ];
    c.bench_function("linear_bump_mixed_alignment", |b| {
        b.iter(|| {
            let mut arena = LinearArena::with_capacity(1 << 20);
            let mut i = 0;
            // Stop before exhaustion; mixed sizes mean we don't fit a fixed
            // count. The benchmark measures bump-with-alignment, not capacity
            // boundary behaviour.
            while i < 512 {
                let (sz, al) = sizes_aligns[i & 5];
                if let Some(slot) = arena.alloc(sz, al) {
                    black_box(slot.as_mut_ptr());
                } else {
                    break;
                }
                i += 1;
            }
        });
    });
}

fn ring_push_steady_state(c: &mut Criterion) {
    c.bench_function("ring_push_steady_state", |b| {
        let mut ring: RingArena<u64> = RingArena::with_capacity(256);
        for i in 0..256u64 {
            ring.push(i);
        }
        // Ring is full; subsequent pushes evict and exercise the hot path.
        let mut counter: u64 = 0;
        b.iter(|| {
            counter = counter.wrapping_add(1);
            ring.push(counter);
        });
    });
}

fn pool_insert_remove_churn(c: &mut Criterion) {
    c.bench_function("pool_insert_remove_churn", |b| {
        b.iter(|| {
            let mut pool: PoolArena<u64> = PoolArena::new();
            let mut handles = Vec::with_capacity(256);
            for i in 0..256u64 {
                handles.push(pool.insert(i));
            }
            // Drain every other handle, refill — the realistic churn pattern.
            for (idx, h) in handles.iter().enumerate() {
                if idx & 1 == 0 {
                    let _ = pool.remove(*h);
                }
            }
            for i in 0..128u64 {
                let _ = pool.insert(i);
            }
            black_box(pool.len());
        });
    });
}

fn general_size_class_walk(c: &mut Criterion) {
    let sizes: [usize; 9] = [16, 32, 64, 128, 256, 512, 1024, 2048, 4096];
    c.bench_function("general_size_class_walk", |b| {
        b.iter(|| {
            let mut arena = GeneralArena::with_capacity(1 << 20);
            let mut ptrs: Vec<*const u8> = Vec::new();
            for &sz in &sizes {
                let slot = arena.alloc(sz).unwrap();
                ptrs.push(slot.as_ptr());
            }
            for &p in &ptrs {
                unsafe { arena.free(p) };
            }
            black_box(arena.stats().used);
        });
    });
}

fn general_fragmentation_pattern(c: &mut Criterion) {
    c.bench_function("general_fragmentation_pattern", |b| {
        b.iter(|| {
            let mut arena = GeneralArena::with_capacity(1 << 20);
            // Allocate a checkerboard of 256-byte blocks, free every other
            // one to force coalescing on the next sweep.
            let mut ptrs: Vec<*const u8> = Vec::with_capacity(128);
            for _ in 0..128 {
                ptrs.push(arena.alloc(256).unwrap().as_ptr());
            }
            for (i, &p) in ptrs.iter().enumerate() {
                if i & 1 == 0 {
                    unsafe { arena.free(p) };
                }
            }
            // Now the size class has 64 free slots; refill them.
            for _ in 0..64 {
                let _ = arena.alloc(256).unwrap();
            }
            black_box(arena.stats().used);
        });
    });
}

fn general_reset_after_churn(c: &mut Criterion) {
    c.bench_function("general_reset_after_churn", |b| {
        b.iter(|| {
            let mut arena = GeneralArena::with_capacity(1 << 20);
            for _ in 0..256 {
                let _ = arena.alloc(64).unwrap();
            }
            arena.reset();
            black_box(arena.stats().used);
        });
    });
}

criterion_group!(
    benches,
    linear_bump_64b,
    linear_bump_mixed_alignment,
    ring_push_steady_state,
    pool_insert_remove_churn,
    general_size_class_walk,
    general_fragmentation_pattern,
    general_reset_after_churn,
);
criterion_main!(benches);
