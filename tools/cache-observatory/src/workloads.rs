//! Cache-behaviour workloads exercising real engine types.
//!
//! Each workload sweeps an array's working-set size across an exponential
//! range. The goal is direct evidence for layout decisions taken elsewhere in
//! the codebase — e.g. ADR-014 (hot/cold component separation) — and a
//! baseline future profiling work can diff against.
//!
//! Determinism: every input is generated from [`engine_core::rng::Rng`] with
//! a fixed seed so a re-run on the same host reproduces the same data layout,
//! and the *only* variable across runs is hardware.

use std::time::Duration;

use engine_core::alloc::LinearArena;
use engine_core::rng::Rng;
use engine_math::{Mat4, Quat, Vec3, vec3};

use crate::timer::{PerfCounters, PerfSample, Stopwatch};

/// One row of the output report.
#[derive(Clone, Debug)]
pub struct Sample {
    /// Working-set size in bytes.
    pub bytes: usize,
    /// Number of elements processed in the inner loop.
    pub elements: usize,
    /// Total wall-clock time spent in the inner loop.
    pub duration: Duration,
    /// Optional cache-miss counts from `perf_event_open`.
    pub perf: PerfSample,
}

impl Sample {
    /// Cost per element, in nanoseconds (the comparable figure across sizes).
    pub fn ns_per_element(&self) -> f64 {
        self.duration.as_secs_f64() * 1.0e9 / self.elements.max(1) as f64
    }
}

/// A report describing one workload sweep.
pub struct Report {
    pub name: &'static str,
    pub description: &'static str,
    pub samples: Vec<Sample>,
}

/// The full set of workloads. Sizes sweep from 4 KiB to 64 MiB doubling, which
/// covers L1 / L2 / LLC / RAM on every modern desktop CPU.
const SIZE_RANGE: &[usize] = &[
    4 * 1024,
    16 * 1024,
    64 * 1024,
    256 * 1024,
    1024 * 1024,
    4 * 1024 * 1024,
    16 * 1024 * 1024,
    64 * 1024 * 1024,
];

/// Number of inner-loop passes to amortise timer overhead.
const PASSES: usize = 4;

/// Black-box wrapper so the optimizer does not delete the workload.
#[inline(never)]
fn black_box<T>(v: T) -> T {
    // `core::hint::black_box` is stable since 1.66; perfect.
    core::hint::black_box(v)
}

/// Streamed sum of `Vec<Vec3>`. The un-padded 12-byte `Vec3` packs three
/// elements per 32 bytes of cache, so this is the canonical sequential read
/// benchmark for the type.
pub fn vec3_array_traversal(mut counters: Option<&mut PerfCounters>) -> Report {
    let mut samples = Vec::new();
    for &bytes in SIZE_RANGE {
        let elements = bytes / core::mem::size_of::<Vec3>();
        let mut rng = Rng::new(0xCAC0_0001, 1);
        let data: Vec<Vec3> = (0..elements)
            .map(|_| vec3(rng.next_f32("x"), rng.next_f32("y"), rng.next_f32("z")))
            .collect();

        warm_up(&data);
        let (duration, perf) = time(counters_reborrow(counters.as_deref_mut()), || {
            let mut acc = Vec3::ZERO;
            for _ in 0..PASSES {
                for v in &data {
                    acc += *v;
                }
            }
            black_box(acc);
        });
        samples.push(Sample {
            bytes,
            elements: elements * PASSES,
            duration,
            perf,
        });
    }
    Report {
        name: "vec3_array_traversal",
        description: "Streamed sum of `Vec<Vec3>` (12-byte un-padded element).",
        samples,
    }
}

/// Hot/cold contrast. Two layouts hold the same `Vec3` data plus a 64-byte
/// "cold" payload, but in one the cold data is interleaved with the hot
/// component and in the other it lives in a parallel array. Direct evidence
/// for ADR-014.
pub fn hot_cold_contrast(mut counters: Option<&mut PerfCounters>) -> (Report, Report) {
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Interleaved {
        hot: Vec3,
        cold: [u8; 64],
    }

    let mut hot_samples = Vec::new();
    let mut cold_samples = Vec::new();

    for &bytes in SIZE_RANGE {
        let elements = bytes / core::mem::size_of::<Interleaved>();

        // --- parallel arrays: only the hot component is touched ---------
        let mut rng = Rng::new(0xCAC0_0002, 1);
        let hot: Vec<Vec3> = (0..elements)
            .map(|_| vec3(rng.next_f32("x"), rng.next_f32("y"), rng.next_f32("z")))
            .collect();
        let _cold_parallel: Vec<[u8; 64]> = (0..elements).map(|_| [0u8; 64]).collect();
        warm_up(&hot);
        let (duration, perf) = time(counters_reborrow(counters.as_deref_mut()), || {
            let mut acc = Vec3::ZERO;
            for _ in 0..PASSES {
                for v in &hot {
                    acc += *v;
                }
            }
            black_box(acc);
        });
        hot_samples.push(Sample {
            bytes,
            elements: elements * PASSES,
            duration,
            perf,
        });

        // --- interleaved: every hot read drags 64 cold bytes through L1 -
        let mut rng = Rng::new(0xCAC0_0002, 1);
        let inter: Vec<Interleaved> = (0..elements)
            .map(|_| Interleaved {
                hot: vec3(rng.next_f32("x"), rng.next_f32("y"), rng.next_f32("z")),
                cold: [0u8; 64],
            })
            .collect();
        warm_up(&inter);
        let (duration, perf) = time(counters_reborrow(counters.as_deref_mut()), || {
            let mut acc = Vec3::ZERO;
            for _ in 0..PASSES {
                for v in &inter {
                    acc += v.hot;
                }
            }
            black_box(acc);
        });
        cold_samples.push(Sample {
            bytes,
            elements: elements * PASSES,
            duration,
            perf,
        });
    }

    (
        Report {
            name: "hot_cold_parallel",
            description: "Parallel-array layout: hot `Vec3` data, cold 64-byte payload separate.",
            samples: hot_samples,
        },
        Report {
            name: "hot_cold_interleaved",
            description: "Interleaved `{Vec3, [u8; 64]}` — every hot read drags cold bytes \
                          through the cache (ADR-014 counter-example).",
            samples: cold_samples,
        },
    )
}

/// Chain of `Mat4` multiplies. A `Mat4` is exactly one 64-byte cache line, so
/// this is the renderer's per-instance transform array in miniature.
pub fn mat4_chain(mut counters: Option<&mut PerfCounters>) -> Report {
    let mut samples = Vec::new();
    for &bytes in SIZE_RANGE {
        let elements = (bytes / core::mem::size_of::<Mat4>()).max(1);
        let mut rng = Rng::new(0xCAC0_0003, 1);
        let data: Vec<Mat4> = (0..elements)
            .map(|_| {
                Mat4::from_trs(
                    vec3(
                        rng.next_f32("tx") * 10.0,
                        rng.next_f32("ty") * 10.0,
                        rng.next_f32("tz") * 10.0,
                    ),
                    Quat::from_axis_angle(Vec3::Y, rng.next_f32("a") * 0.1),
                    Vec3::ONE,
                )
            })
            .collect();
        warm_up(&data);
        let (duration, perf) = time(counters_reborrow(counters.as_deref_mut()), || {
            let mut acc = Mat4::IDENTITY;
            for _ in 0..PASSES {
                for m in &data {
                    acc = acc * *m;
                }
            }
            black_box(acc);
        });
        samples.push(Sample {
            bytes,
            elements: elements * PASSES,
            duration,
            perf,
        });
    }
    Report {
        name: "mat4_chain",
        description: "Sequential `Mat4` multiply chain (one cache line per element).",
        samples,
    }
}

/// Pointer-chase a `LinearArena` via a deterministic-seed shuffled index list,
/// observing the cost at each cache level.
pub fn linear_arena_random_reads(mut counters: Option<&mut PerfCounters>) -> Report {
    let mut samples = Vec::new();
    for &bytes in SIZE_RANGE {
        // Fill the arena with `u64` records — small enough that the index
        // shuffle dominates the access pattern.
        let elements = (bytes / 8).max(1);
        let mut arena = LinearArena::with_capacity(elements * 8);
        for i in 0..elements {
            let slot = arena.alloc(8, 8).expect("arena capacity sized to fit");
            slot.copy_from_slice(&(i as u64).to_le_bytes());
        }

        // Deterministic Fisher-Yates over a 0..elements index list.
        let mut rng = Rng::new(0xCAC0_0004, 1);
        let mut indices: Vec<u32> = (0..elements as u32).collect();
        for i in (1..elements).rev() {
            let j = (rng.next_u64("shuf") as usize) % (i + 1);
            indices.swap(i, j);
        }

        // Read-only view of the arena's used region (we never mutate after
        // the fill loop above).
        let base = arena.used_bytes();

        let (duration, perf) = time(counters_reborrow(counters.as_deref_mut()), || {
            let mut acc: u64 = 0;
            for _ in 0..PASSES {
                for &i in &indices {
                    let off = i as usize * 8;
                    acc = acc
                        .wrapping_add(u64::from_le_bytes(base[off..off + 8].try_into().unwrap()));
                }
            }
            black_box(acc);
        });
        samples.push(Sample {
            bytes,
            elements: elements * PASSES,
            duration,
            perf,
        });
    }
    Report {
        name: "linear_arena_random_reads",
        description: "Pointer-chase a `LinearArena` via a deterministic shuffled index list.",
        samples,
    }
}

// --- helpers -----------------------------------------------------------

#[inline(never)]
fn warm_up<T>(buf: &[T]) {
    // Touch the first byte of every cache line so the first measured pass is
    // not skewed by page-fault overhead.
    let bytes = unsafe {
        std::slice::from_raw_parts(buf.as_ptr() as *const u8, core::mem::size_of_val(buf))
    };
    let mut acc: u8 = 0;
    let stride = 64;
    let mut i = 0;
    while i < bytes.len() {
        acc = acc.wrapping_add(bytes[i]);
        i += stride;
    }
    black_box(acc);
}

#[inline]
fn time<F: FnOnce()>(counters: Option<&mut PerfCounters>, workload: F) -> (Duration, PerfSample) {
    let mut perf = PerfSample::default();
    if let Some(c) = counters {
        c.start();
        let sw = Stopwatch::start();
        workload();
        let dur = sw.elapsed();
        perf = c.snapshot();
        (dur, perf)
    } else {
        let sw = Stopwatch::start();
        workload();
        (sw.elapsed(), perf)
    }
}

#[inline]
fn counters_reborrow(c: Option<&mut PerfCounters>) -> Option<&mut PerfCounters> {
    c
}
