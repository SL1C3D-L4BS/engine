//! `sampling-profiler` — engine sampling profiler CLI (ADR-030).
//!
//! Drives [`engine_telemetry::SamplingProfiler`] against built-in CPU-bound
//! workloads and emits folded-stack output compatible with Brendan
//! Gregg's `flamegraph.pl`. Self-overhead is reported on stderr so it
//! does not pollute the pipeline:
//!
//! ```text
//! cargo run --release -p sampling-profiler -- --rate-hz 199 --duration-s 2 --workload arena_alloc
//! cargo run --release -p sampling-profiler -- --workload spinner | flamegraph.pl > spinner.svg
//! ```
//!
//! Owned argument parsing (R-02). Linux-only via the perf path the
//! profiler sits on; on macOS / Windows the binary still runs but
//! `SamplingProfiler::try_attach` returns `Ok(None)` and we emit a
//! one-line warning instead of a profile.

use std::process::ExitCode;
use std::time::{Duration, Instant};

use engine_core::alloc::LinearArena;
use engine_telemetry::SamplingProfiler;

#[derive(Debug)]
struct Args {
    rate_hz: u32,
    duration_s: u32,
    workload: String,
    help: bool,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            rate_hz: 99,
            duration_s: 1,
            workload: "spinner".into(),
            help: false,
        }
    }
}

const USAGE: &str = "\
sampling-profiler — engine sampling profiler CLI

USAGE:
    sampling-profiler [--rate-hz <N>] [--duration-s <N>] [--workload <NAME>] [--help]

OPTIONS:
    --rate-hz <99|199|499|997>   Sampling rate in Hz (default 99).
    --duration-s <N>             Workload duration in seconds (default 1).
    --workload <NAME>            Built-in workload. Valid names:
                                   spinner       — tight ALU loop
                                   arena_alloc   — engine-core LinearArena hot loop
    -h, --help                   Print this message.

OUTPUT:
    Folded-stack lines on stdout (frame;frame;frame N), one per unique
    call chain — the format Brendan Gregg's flamegraph.pl reads.
    Self-overhead statistics on stderr.
";

fn parse_args() -> Result<Args, String> {
    let mut out = Args::default();
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--rate-hz" => {
                let v = it.next().ok_or("--rate-hz requires a value")?;
                out.rate_hz = v.parse().map_err(|_| format!("bad --rate-hz: {v}"))?;
            }
            "--duration-s" => {
                let v = it.next().ok_or("--duration-s requires a value")?;
                out.duration_s = v.parse().map_err(|_| format!("bad --duration-s: {v}"))?;
            }
            "--workload" => {
                out.workload = it.next().ok_or("--workload requires a value")?;
            }
            "-h" | "--help" => out.help = true,
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(out)
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    if args.help {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }

    let profiler = match SamplingProfiler::try_attach(args.rate_hz) {
        Ok(Some(p)) => p,
        Ok(None) => {
            eprintln!(
                "sampling-profiler: perf_event_open unavailable on this host \
                 (perf_event_paranoid > 2 without CAP_PERFMON, or not Linux); \
                 emitting an empty profile."
            );
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("sampling-profiler: unexpected error attaching: {e}");
            return ExitCode::from(1);
        }
    };

    // Self-overhead bookkeeping: total wall-clock to drive the workload.
    let t_start = Instant::now();

    let deadline = Duration::from_secs(args.duration_s as u64);
    match args.workload.as_str() {
        "spinner" => workload_spinner(deadline),
        "arena_alloc" => workload_arena_alloc(deadline),
        other => {
            eprintln!("sampling-profiler: unknown workload `{other}`\n\n{USAGE}");
            return ExitCode::from(2);
        }
    }

    let workload_wall = t_start.elapsed();
    let folded = profiler.finish();

    // Folded-stack lines to stdout.
    print!("{}", folded.folded_text());

    // Self-overhead summary to stderr.
    eprintln!(
        "sampling-profiler: {} stacks, {} samples (dropped {}), workload {:?}, rate {} Hz",
        folded.stacks.len(),
        folded.samples_seen,
        folded.samples_dropped,
        workload_wall,
        args.rate_hz
    );
    let total_samples = folded.total();
    if total_samples > 0 {
        let expected = (args.rate_hz as u64) * (args.duration_s as u64);
        let coverage = total_samples as f64 / expected as f64;
        eprintln!(
            "sampling-profiler: sample coverage {:.1}% \
             ({} captured / {} expected at {} Hz × {} s)",
            coverage * 100.0,
            total_samples,
            expected,
            args.rate_hz,
            args.duration_s
        );
    }

    ExitCode::SUCCESS
}

fn workload_spinner(deadline: Duration) {
    let mut acc: u64 = 1;
    let start = Instant::now();
    loop {
        for i in 0..1_000_000u64 {
            acc = acc.wrapping_mul(2_654_435_761).wrapping_add(i ^ acc);
        }
        if start.elapsed() >= deadline {
            break;
        }
    }
    std::hint::black_box(acc);
}

fn workload_arena_alloc(deadline: Duration) {
    let start = Instant::now();
    let mut sink: u64 = 0;
    while start.elapsed() < deadline {
        let mut arena = LinearArena::with_capacity(1 << 20);
        for _ in 0..1024 {
            let slot = arena.alloc(64, 8).unwrap();
            // Touch one byte so the optimizer can't elide.
            sink = sink.wrapping_add(slot.as_mut_ptr() as u64);
        }
    }
    std::hint::black_box(sink);
}
