//! Criterion benches for the engine-core Robin Hood hash map.
//!
//! Run with `just bench` (or `cargo bench -p engine-core --bench collections`).
//! Bench numbers are too runner-noisy to gate CI on; commit refreshed numbers
//! to `docs/observatory/hashmap-baseline.md` after every meaningful change
//! (ADR-028).
//!
//! Three comparison points:
//!
//! - `std::collections::HashMap` with the default `RandomState` (SwissTable +
//!   SipHash) — the reference implementation we are replacing.
//! - `std::collections::HashMap` with our owned [`FastHasher`] — isolates
//!   the SwissTable probing strategy from the hash function.
//! - Our [`HashMap`] with [`FastHasher`] (the production default).

// Criterion's `criterion_group!` macro emits an undocumented `pub fn`;
// suppress the workspace-wide `missing_docs = "deny"` here.
#![allow(missing_docs)]

use std::collections::HashMap as StdHashMap;
use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use engine_core::collections::{FastHasher, HashMap};

const N: u32 = 4096;

fn workload_keys() -> Vec<u32> {
    let mut state: u64 = 0xDEAD_BEEF_CAFE_F00D;
    (0..N)
        .map(|_| {
            state = state.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1);
            state as u32
        })
        .collect()
}

fn insert_grow_ours(c: &mut Criterion) {
    let keys = workload_keys();
    c.bench_function("hashmap_insert_grow_ours", |b| {
        b.iter(|| {
            let mut m: HashMap<u32, u32> = HashMap::new();
            for &k in &keys {
                m.insert(black_box(k), black_box(k));
            }
            black_box(m.len())
        });
    });
}

fn insert_grow_std_siphash(c: &mut Criterion) {
    let keys = workload_keys();
    c.bench_function("hashmap_insert_grow_std_siphash", |b| {
        b.iter(|| {
            let mut m: StdHashMap<u32, u32> = StdHashMap::new();
            for &k in &keys {
                m.insert(black_box(k), black_box(k));
            }
            black_box(m.len())
        });
    });
}

fn insert_grow_std_fast(c: &mut Criterion) {
    let keys = workload_keys();
    c.bench_function("hashmap_insert_grow_std_fasthasher", |b| {
        b.iter(|| {
            let mut m: StdHashMap<u32, u32, FastHasher> =
                StdHashMap::with_hasher(FastHasher::new());
            for &k in &keys {
                m.insert(black_box(k), black_box(k));
            }
            black_box(m.len())
        });
    });
}

fn get_hit_ours(c: &mut Criterion) {
    let keys = workload_keys();
    let mut m: HashMap<u32, u32> = HashMap::with_capacity(N as usize);
    for &k in &keys {
        m.insert(k, k);
    }
    c.bench_function("hashmap_get_hit_ours", |b| {
        b.iter(|| {
            let mut sum: u64 = 0;
            for &k in &keys {
                sum += *m.get(&black_box(k)).unwrap() as u64;
            }
            black_box(sum)
        });
    });
}

fn get_hit_std_siphash(c: &mut Criterion) {
    let keys = workload_keys();
    let mut m: StdHashMap<u32, u32> = StdHashMap::with_capacity(N as usize);
    for &k in &keys {
        m.insert(k, k);
    }
    c.bench_function("hashmap_get_hit_std_siphash", |b| {
        b.iter(|| {
            let mut sum: u64 = 0;
            for &k in &keys {
                sum += *m.get(&black_box(k)).unwrap() as u64;
            }
            black_box(sum)
        });
    });
}

fn get_hit_std_fast(c: &mut Criterion) {
    let keys = workload_keys();
    let mut m: StdHashMap<u32, u32, FastHasher> =
        StdHashMap::with_capacity_and_hasher(N as usize, FastHasher::new());
    for &k in &keys {
        m.insert(k, k);
    }
    c.bench_function("hashmap_get_hit_std_fasthasher", |b| {
        b.iter(|| {
            let mut sum: u64 = 0;
            for &k in &keys {
                sum += *m.get(&black_box(k)).unwrap() as u64;
            }
            black_box(sum)
        });
    });
}

fn get_miss_ours(c: &mut Criterion) {
    let keys = workload_keys();
    let misses: Vec<u32> = keys.iter().map(|k| k.wrapping_add(1)).collect();
    let mut m: HashMap<u32, u32> = HashMap::with_capacity(N as usize);
    for &k in &keys {
        m.insert(k, k);
    }
    c.bench_function("hashmap_get_miss_ours", |b| {
        b.iter(|| {
            let mut found: u64 = 0;
            for &k in &misses {
                if m.contains_key(&black_box(k)) {
                    found += 1;
                }
            }
            black_box(found)
        });
    });
}

fn get_miss_std_siphash(c: &mut Criterion) {
    let keys = workload_keys();
    let misses: Vec<u32> = keys.iter().map(|k| k.wrapping_add(1)).collect();
    let mut m: StdHashMap<u32, u32> = StdHashMap::with_capacity(N as usize);
    for &k in &keys {
        m.insert(k, k);
    }
    c.bench_function("hashmap_get_miss_std_siphash", |b| {
        b.iter(|| {
            let mut found: u64 = 0;
            for &k in &misses {
                if m.contains_key(&black_box(k)) {
                    found += 1;
                }
            }
            black_box(found)
        });
    });
}

fn remove_ours(c: &mut Criterion) {
    let keys = workload_keys();
    c.bench_function("hashmap_remove_ours", |b| {
        b.iter(|| {
            let mut m: HashMap<u32, u32> = HashMap::with_capacity(N as usize);
            for &k in &keys {
                m.insert(k, k);
            }
            for &k in &keys {
                m.remove(&black_box(k));
            }
            black_box(m.len())
        });
    });
}

fn remove_std_siphash(c: &mut Criterion) {
    let keys = workload_keys();
    c.bench_function("hashmap_remove_std_siphash", |b| {
        b.iter(|| {
            let mut m: StdHashMap<u32, u32> = StdHashMap::with_capacity(N as usize);
            for &k in &keys {
                m.insert(k, k);
            }
            for &k in &keys {
                m.remove(&black_box(k));
            }
            black_box(m.len())
        });
    });
}

criterion_group!(
    benches,
    insert_grow_ours,
    insert_grow_std_siphash,
    insert_grow_std_fast,
    get_hit_ours,
    get_hit_std_siphash,
    get_hit_std_fast,
    get_miss_ours,
    get_miss_std_siphash,
    remove_ours,
    remove_std_siphash,
);
criterion_main!(benches);
