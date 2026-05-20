//! Sampling profiler — folded-stack consumer (ADR-030).
//!
//! The producer side ([`engine_platform::sampler::Sampler`]) opens a
//! per-thread `perf_event_open` fd and an mmap'd ring buffer; this module
//! is the consumer side. It folds each captured call chain into a
//! `Vec<u64> -> u64` count and exposes the result as
//! [`FoldedStacks`], the same shape Brendan Gregg's `flamegraph.pl`
//! script consumes (one line per unique stack, `frame;frame;frame N`).
//!
//! On non-Linux platforms [`SamplingProfiler::try_attach`] returns
//! `Ok(None)`; the engine never refuses to start because the profiler
//! could not attach.

use engine_core::collections::HashMap;
use engine_core::telemetry::{Signal, record};
use engine_platform::sampler::{Sample, Sampler};
use std::hash::{BuildHasher, Hash, Hasher};
use std::io;

/// A single folded-stack record: instruction-pointer chain (leaf first)
/// and the number of samples that landed on it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FoldedStack {
    /// Instruction-pointer chain, leaf first.
    pub ips: Vec<u64>,
    /// Number of samples that produced this exact chain.
    pub count: u64,
}

/// The output of one sampling session.
#[derive(Clone, Debug, Default)]
pub struct FoldedStacks {
    /// Folded stacks in arbitrary order. Use [`FoldedStacks::sort_by_count`]
    /// for the conventional "hottest first" ordering.
    pub stacks: Vec<FoldedStack>,
    /// Total samples seen by the producer.
    pub samples_seen: u64,
    /// Samples dropped (ring overflow / non-sample records).
    pub samples_dropped: u64,
}

impl FoldedStacks {
    /// Total count across every recorded stack.
    pub fn total(&self) -> u64 {
        self.stacks.iter().map(|s| s.count).sum()
    }

    /// Sort descending by count.
    pub fn sort_by_count(&mut self) {
        self.stacks.sort_by_key(|s| std::cmp::Reverse(s.count));
    }

    /// Render in the `frame;frame;frame N` line-per-stack format that
    /// Brendan Gregg's `flamegraph.pl` consumes. Frames are hex
    /// instruction-pointer addresses — symbolization is a CLI-level
    /// concern (the profiler tool reads `/proc/self/maps` and the ELF
    /// dynamic symbol table for that).
    pub fn folded_text(&self) -> String {
        let mut out = String::new();
        for stack in &self.stacks {
            // Spec layout for flamegraph.pl: outermost frame first,
            // separated by `;`, count last.
            for (i, ip) in stack.ips.iter().rev().enumerate() {
                if i > 0 {
                    out.push(';');
                }
                out.push_str(&format!("0x{ip:x}"));
            }
            out.push(' ');
            out.push_str(&stack.count.to_string());
            out.push('\n');
        }
        out
    }
}

/// Self-contained sampling-profiler session attached to the calling thread.
///
/// Open one of these on each thread you want sampled. Drop or call
/// [`SamplingProfiler::finish`] to stop sampling and harvest the folded
/// stacks.
pub struct SamplingProfiler {
    sampler: Sampler,
    pending: Vec<Sample>,
}

impl SamplingProfiler {
    /// Tries to attach a sampler to the calling thread at `rate_hz`
    /// samples per second.
    ///
    /// Returns `Ok(None)` if the kernel refuses (perf_event_paranoid > 2
    /// without CAP_PERFMON) or the platform is unsupported (macOS,
    /// Windows). Returns `Err` only for unexpected failures.
    pub fn try_attach(rate_hz: u32) -> io::Result<Option<Self>> {
        match Sampler::try_open(rate_hz)? {
            None => Ok(None),
            Some(mut sampler) => {
                sampler.start();
                Ok(Some(Self {
                    sampler,
                    pending: Vec::new(),
                }))
            }
        }
    }

    /// Drains pending samples into the in-progress fold without stopping
    /// the producer. Call this periodically on long sessions if the ring
    /// is undersized for the inter-drain interval.
    pub fn drain(&mut self) {
        self.sampler.drain(&mut self.pending);
    }

    /// Stops sampling and returns the folded stacks.
    pub fn finish(mut self) -> FoldedStacks {
        self.sampler.stop();
        self.sampler.drain(&mut self.pending);

        // Fold by the full IP chain. The HashMap uses the default
        // FastHasher (ADR-028) — the keys are Vec<u64>, which the
        // hasher folds u64-wise.
        let mut folded: HashMap<Vec<u64>, u64> = HashMap::new();
        for s in self.pending.drain(..) {
            *folded.entry_or_zero(s.ips) += 1;
        }

        let samples_seen = self.sampler.seen();
        let samples_dropped = self.sampler.dropped();

        let mut stacks: Vec<FoldedStack> = folded
            .into_iter()
            .map(|(ips, count)| FoldedStack { ips, count })
            .collect();
        stacks.sort_by_key(|s| std::cmp::Reverse(s.count));

        // Emit one Signal::Sample per unique stack so downstream
        // telemetry consumers (collector / ipc / metrics endpoint) see
        // the sampling activity without draining the ring directly
        // (ADR-030). The stack identifier is the FastHasher digest of
        // the IP chain — collisions are extremely unlikely for typical
        // chain depths (≤ 32 frames).
        let id_hasher = engine_core::collections::FastHasher::new();
        for stack in &stacks {
            let mut h = id_hasher.build_hasher();
            for ip in &stack.ips {
                ip.hash(&mut h);
            }
            let stack_id = h.finish();
            record(Signal::Sample {
                stack_id,
                count: stack.count,
            });
        }

        FoldedStacks {
            stacks,
            samples_seen,
            samples_dropped,
        }
    }
}

/// Convenience extension: `entry().or_insert(0)`-style helper for the
/// folded-stack count map. Local to this module because the production
/// [`HashMap`] does not ship a full `Entry` API (ADR-028).
trait EntryOrZero<K, V> {
    fn entry_or_zero(&mut self, key: K) -> &mut V;
}

impl<K: std::hash::Hash + Eq + Clone, S: std::hash::BuildHasher> EntryOrZero<K, u64>
    for HashMap<K, u64, S>
{
    fn entry_or_zero(&mut self, key: K) -> &mut u64 {
        if !self.contains_key(&key) {
            self.insert(key.clone(), 0);
        }
        self.get_mut(&key).expect("entry was just inserted")
    }
}
