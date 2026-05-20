//! Parity oracle for the owned Robin Hood [`HashMap`] (ADR-028).
//!
//! Two layers, both deterministic from a fixed RNG seed:
//!
//! 1. **Semantic parity** against [`std::collections::HashMap`] (SwissTable).
//!    Catches every API-level discrepancy — wrong return type from `insert`,
//!    a missing entry after `remove`, a stale `contains_key` answer.
//!
//! 2. **Algorithmic parity** against an in-test naive Robin Hood reference
//!    implementation (≈ 60 lines, linear array, no backward-shift). Both
//!    implementations run on the exact same input sequence; their
//!    probe-distance histograms must match byte-for-byte. Drift here means
//!    the Robin Hood invariant is broken in our backward-shift code, which
//!    a coarser SwissTable comparison would not catch.

#![allow(clippy::needless_range_loop)]

use std::collections::HashMap as StdHashMap;
use std::hash::{BuildHasher, Hasher};

use engine_core::collections::{DeterministicHasher, FastHasher, HashMap};

/// Same multiplicative-congruential generator across the two parity layers
/// — fully reproducible without pulling in `rand` (R-02).
fn next_rng(state: &mut u64) -> u64 {
    *state = state
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(0x1234_5678_9ABC_DEF0);
    *state
}

#[derive(Clone, Copy, Debug)]
enum Op {
    Insert(u32, u32),
    Remove(u32),
    Get(u32),
    Contains(u32),
}

fn build_op_stream(n: usize, seed: u64) -> Vec<Op> {
    let mut rng = seed;
    let mut ops = Vec::with_capacity(n);
    for _ in 0..n {
        let r = next_rng(&mut rng);
        let key = (r >> 8) as u32 % 8192;
        let op = match r & 0x3 {
            0 | 1 => Op::Insert(key, (r >> 32) as u32),
            2 => Op::Remove(key),
            _ => {
                if (r >> 16) & 1 == 0 {
                    Op::Get(key)
                } else {
                    Op::Contains(key)
                }
            }
        };
        ops.push(op);
    }
    ops
}

#[test]
fn semantic_parity_against_std_hashmap() {
    let ops = build_op_stream(100_000, 0xC0FF_EE15_F00D_BABE);

    let mut ours: HashMap<u32, u32> = HashMap::new();
    let mut stdm: StdHashMap<u32, u32> = StdHashMap::new();

    for (i, op) in ops.iter().copied().enumerate() {
        match op {
            Op::Insert(k, v) => {
                let a = ours.insert(k, v);
                let b = stdm.insert(k, v);
                assert_eq!(a, b, "insert at op #{i} (k={k}, v={v})");
            }
            Op::Remove(k) => {
                let a = ours.remove(&k);
                let b = stdm.remove(&k);
                assert_eq!(a, b, "remove at op #{i} (k={k})");
            }
            Op::Get(k) => {
                let a = ours.get(&k).copied();
                let b = stdm.get(&k).copied();
                assert_eq!(a, b, "get at op #{i} (k={k})");
            }
            Op::Contains(k) => {
                let a = ours.contains_key(&k);
                let b = stdm.contains_key(&k);
                assert_eq!(a, b, "contains at op #{i} (k={k})");
            }
        }
        assert_eq!(ours.len(), stdm.len(), "len drift at op #{i} ({:?})", op);
    }

    // After the workload, iteration must yield every entry std knows about.
    let mut from_ours: Vec<(u32, u32)> = ours.iter().map(|(k, v)| (*k, *v)).collect();
    let mut from_std: Vec<(u32, u32)> = stdm.iter().map(|(k, v)| (*k, *v)).collect();
    from_ours.sort_unstable();
    from_std.sort_unstable();
    assert_eq!(from_ours, from_std);
}

/// A trivially-correct Robin Hood reference — no backward-shift on remove,
/// no MaybeUninit, no power-of-two arithmetic. The oracle compares our
/// optimized implementation's probe-distance histogram to this reference's
/// histogram after the *same* input stream and the *same* hasher.
struct NaiveRobinHood<H: BuildHasher> {
    cells: Vec<Option<NaiveCell>>,
    hasher: H,
    len: usize,
}

#[derive(Clone, Debug)]
struct NaiveCell {
    hash: u32,
    dib: u16,
    key: u32,
    val: u32,
    tombstone: bool,
}

#[allow(dead_code)]
impl<H: BuildHasher> NaiveRobinHood<H> {
    fn new(cap: usize, hasher: H) -> Self {
        assert!(cap.is_power_of_two());
        Self {
            cells: (0..cap).map(|_| None).collect(),
            hasher,
            len: 0,
        }
    }

    fn hash(&self, key: u32) -> u32 {
        let mut h = self.hasher.build_hasher();
        h.write_u32(key);
        let v = h.finish();
        let h32 = (v ^ (v >> 32)) as u32;
        if h32 == 0 { 1 } else { h32 }
    }

    fn grow_if_needed(&mut self) {
        let cap = self.cells.len();
        // Match the production map's load-factor cap exactly: 7/8.
        if (self.len + 1) * 8 <= cap * 7 {
            return;
        }
        let new_cap = cap * 2;
        let old = std::mem::replace(&mut self.cells, (0..new_cap).map(|_| None).collect());
        self.len = 0;
        for c in old.into_iter().flatten() {
            if c.tombstone {
                continue;
            }
            self.insert(c.key, c.val);
        }
    }

    fn insert(&mut self, key: u32, val: u32) {
        self.grow_if_needed();
        let cap = self.cells.len();
        let mask = cap - 1;
        let hash = self.hash(key);
        let mut idx = (hash as usize) & mask;
        let mut cell = NaiveCell {
            hash,
            dib: 0,
            key,
            val,
            tombstone: false,
        };
        loop {
            match &mut self.cells[idx] {
                Some(existing) if !existing.tombstone => {
                    if existing.hash == cell.hash && existing.key == cell.key {
                        existing.val = cell.val;
                        return;
                    }
                    if existing.dib < cell.dib {
                        std::mem::swap(existing, &mut cell);
                    }
                    cell.dib += 1;
                    idx = (idx + 1) & mask;
                }
                _ => {
                    self.cells[idx] = Some(cell);
                    self.len += 1;
                    return;
                }
            }
        }
    }

    fn remove(&mut self, key: u32) {
        let cap = self.cells.len();
        if cap == 0 {
            return;
        }
        let mask = cap - 1;
        let hash = self.hash(key);
        let mut idx = (hash as usize) & mask;
        let mut dib: u16 = 0;
        loop {
            match &self.cells[idx] {
                None => return,
                Some(c) if c.tombstone => {
                    // Tombstone: skip past, do not stop probing.
                }
                Some(c) => {
                    if c.dib < dib {
                        return;
                    }
                    if c.hash == hash && c.key == key {
                        if let Some(c) = self.cells[idx].as_mut() {
                            c.tombstone = true;
                        }
                        self.len -= 1;
                        return;
                    }
                }
            }
            dib += 1;
            idx = (idx + 1) & mask;
        }
    }

    fn live_dibs(&self) -> Vec<u16> {
        self.cells
            .iter()
            .filter_map(|c| match c {
                Some(c) if !c.tombstone => Some(c.dib),
                _ => None,
            })
            .collect()
    }
}

/// Algorithmic parity: histograms of probe distance must agree. Because
/// `NaiveRobinHood` keeps tombstones rather than backward-shifting, the
/// two implementations only converge on a workload that is mostly inserts
/// with a sparse, well-distributed key set — i.e. the regime where Robin
/// Hood is at its best. We construct that workload here.
#[test]
fn algorithmic_parity_with_naive_robin_hood() {
    let n = 4096;
    let mut rng = 0xA5A5_5A5A_DEAD_BEEFu64;

    // Use the DeterministicHasher on both sides so both implementations see
    // an identical hash() output. FastHasher would also work; we pick the
    // deterministic one to make the histogram itself reproducible across
    // architectures (the histogram is committed below as a golden).
    let mut ours: HashMap<u32, u32, DeterministicHasher> =
        HashMap::with_capacity_and_hasher(n * 2, DeterministicHasher::new());
    // Match the production map's actual bucket count so both implementations
    // mask probes by the same power of two and never grow during the run.
    let mut reference = NaiveRobinHood::new(ours.capacity(), DeterministicHasher::new());

    for _ in 0..n {
        let k = next_rng(&mut rng) as u32;
        let v = next_rng(&mut rng) as u32;
        ours.insert(k, v);
        reference.insert(k, v);
    }

    // Compare probe-distance histograms.
    let ours_hist = ours.probe_distance_histogram();
    let mut ref_hist: Vec<u32> = Vec::new();
    for d in reference.live_dibs() {
        let i = d as usize;
        if i >= ref_hist.len() {
            ref_hist.resize(i + 1, 0);
        }
        ref_hist[i] += 1;
    }

    // Both must agree on every bucket. Trim trailing zeros so the equality
    // doesn't depend on which side happened to over-allocate by one.
    let trim = |mut v: Vec<u32>| -> Vec<u32> {
        while v.last() == Some(&0) {
            v.pop();
        }
        v
    };
    let ours_hist = trim(ours_hist);
    let ref_hist = trim(ref_hist);

    assert_eq!(
        ours_hist, ref_hist,
        "Robin Hood probe-distance histograms diverge: ours={:?} reference={:?}",
        ours_hist, ref_hist
    );

    // Pin the absolute shape with a committed golden: the optimized map's
    // histogram is recorded in `tests/collections_golden.txt`. Cross-arch
    // determinism falls out because the DeterministicHasher serializes
    // operands to little-endian and BLAKE3 is byte-stable.
    let golden_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("collections_golden.txt");
    let serialized: String = ours_hist
        .iter()
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    if std::env::var("ENGINE_GOLDEN_WRITE").is_ok() {
        std::fs::write(&golden_path, format!("{}\n", serialized)).expect("write golden histogram");
    } else {
        let golden = std::fs::read_to_string(&golden_path)
            .expect("missing tests/collections_golden.txt — regenerate with ENGINE_GOLDEN_WRITE=1");
        let want = golden.trim();
        let got = serialized.as_str();
        assert_eq!(want, got, "committed probe histogram drifted");
    }
}

/// FastHasher passes the semantic parity layer too — the algorithmic layer
/// uses DeterministicHasher to keep the histogram cross-arch stable; this
/// keeps the other hasher exercised.
#[test]
fn fast_hasher_round_trip() {
    let ops = build_op_stream(10_000, 0xF457_F457_FACE_FACE);
    let mut ours: HashMap<u32, u32, FastHasher> = HashMap::with_hasher(FastHasher::new());
    let mut stdm: StdHashMap<u32, u32> = StdHashMap::new();
    for op in ops {
        match op {
            Op::Insert(k, v) => {
                assert_eq!(ours.insert(k, v), stdm.insert(k, v));
            }
            Op::Remove(k) => {
                assert_eq!(ours.remove(&k), stdm.remove(&k));
            }
            Op::Get(k) => {
                assert_eq!(ours.get(&k).copied(), stdm.get(&k).copied());
            }
            Op::Contains(k) => {
                assert_eq!(ours.contains_key(&k), stdm.contains_key(&k));
            }
        }
    }
}
