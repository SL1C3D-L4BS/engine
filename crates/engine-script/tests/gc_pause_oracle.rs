//! GC pause-time oracle (PR 2, ADR-035).
//!
//! The Phase 4 spec calls for `p99 < 1 ms`, `max < 5 ms` over a
//! 100k-object steady-state with 10k churn per tick for 1k ticks.
//! PR 2 ships the single-generation `Heap` (see `gc/mod.rs`) so the
//! oracle runs **informational**: it logs the histogram but does not
//! fail. The hard CI gate lands with the generational follow-up.
//!
//! The test only runs in `--release`; debug-build pause times do not
//! reflect the dispatch loop's optimised cost.

use engine_script::gc::{Heap, Obj};
use engine_script::vm::Value;
use std::time::Instant;

const STEADY_STATE: u32 = 100_000;
const CHURN_PER_TICK: u32 = 10_000;
const TICKS: u32 = 1_000;

#[test]
fn gc_pause_under_budget_informational() {
    if cfg!(debug_assertions) {
        eprintln!("gc_pause_oracle skipped in debug builds — use --release");
        return;
    }
    let mut heap = Heap::with_default_config();
    let mut handles = Vec::with_capacity(STEADY_STATE as usize);
    for i in 0..STEADY_STATE {
        handles.push(heap.alloc(Obj::Array(vec![Value::Int(i as i64)])));
    }

    let mut samples_us: Vec<u64> = Vec::with_capacity(TICKS as usize);
    for _ in 0..TICKS {
        // Churn: drop 10k oldest, allocate 10k fresh.
        let drop_n = CHURN_PER_TICK.min(handles.len() as u32) as usize;
        handles.drain(0..drop_n);
        for _ in 0..CHURN_PER_TICK {
            handles.push(heap.alloc(Obj::Array(vec![Value::Int(0)])));
        }
        // Build the root set from current handles.
        let roots: Vec<Value> = handles.iter().map(|h| Value::Array(*h)).collect();
        let start = Instant::now();
        heap.collect(&roots);
        samples_us.push(start.elapsed().as_micros() as u64);
    }

    samples_us.sort_unstable();
    let p99 = samples_us[(TICKS as usize * 99) / 100];
    let max = *samples_us.last().unwrap();
    eprintln!(
        "gc_pause_oracle (informational, single-gen): p99 = {} µs, max = {} µs",
        p99, max
    );
    // ADR-035 follow-up wires this as a hard gate (p99 < 1000, max <
    // 5000) when the generational variant lands.
}
