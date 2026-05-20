//! `cache-observatory` — measures cache behaviour for the engine's hot data
//! types (spec XXI, "SILICON → C").
//!
//! Run from the workspace root:
//!
//! ```text
//! cargo run --release -p cache-observatory
//! cargo run --release -p cache-observatory -- --with-perf-counters
//! cargo run --release -p cache-observatory -- --only vec3_array_traversal
//! ```
//!
//! Output is markdown on stdout; redirect to
//! `docs/observatory/cache-baseline.md` to refresh the committed baseline.

#[cfg(target_os = "linux")]
mod perf;
mod report;
mod timer;
mod workloads;

use std::process::ExitCode;

use timer::PerfCounters;

#[derive(Debug, Default)]
struct Args {
    with_perf_counters: bool,
    only: Option<String>,
    help: bool,
}

const USAGE: &str = "\
cache-observatory — engine cache behaviour baseline

USAGE:
    cache-observatory [--with-perf-counters] [--only <workload>] [--help]

OPTIONS:
    --with-perf-counters   Open kernel perf-event-open L1/L2/LLC miss counters.
                           Falls back silently if perf_event_paranoid blocks it.
    --only <workload>      Run only the named workload. Valid names:
                             vec3_array_traversal
                             hot_cold
                             mat4_chain
                             linear_arena_random_reads
    -h, --help             Print this message.
";

fn parse_args() -> Result<Args, String> {
    let mut out = Args::default();
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--with-perf-counters" => out.with_perf_counters = true,
            "--only" => {
                out.only = Some(
                    it.next()
                        .ok_or_else(|| "--only requires a workload name".to_string())?,
                );
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

    let mut counters: Option<PerfCounters> = None;
    if args.with_perf_counters {
        match PerfCounters::try_open() {
            Ok(Some(c)) => counters = Some(c),
            Ok(None) => eprintln!(
                "note: --with-perf-counters requested but perf_event_open is not \
                 wired in on this build; continuing with wall-clock only."
            ),
            Err(e) => eprintln!(
                "note: --with-perf-counters: kernel refused perf_event_open ({e}); \
                 continuing with wall-clock only."
            ),
        }
    }

    let mut reports = Vec::new();
    let only = args.only.as_deref();
    if matches_workload(only, "vec3_array_traversal") {
        reports.push(workloads::vec3_array_traversal(counters.as_mut()));
    }
    if matches_workload(only, "hot_cold") {
        let (hot, inter) = workloads::hot_cold_contrast(counters.as_mut());
        reports.push(hot);
        reports.push(inter);
    }
    if matches_workload(only, "mat4_chain") {
        reports.push(workloads::mat4_chain(counters.as_mut()));
    }
    if matches_workload(only, "linear_arena_random_reads") {
        reports.push(workloads::linear_arena_random_reads(counters.as_mut()));
    }

    if reports.is_empty() {
        eprintln!("error: --only filtered out every workload\n\n{USAGE}");
        return ExitCode::from(2);
    }

    print!("{}", report::render(&reports));
    ExitCode::SUCCESS
}

fn matches_workload(filter: Option<&str>, name: &str) -> bool {
    match filter {
        None => true,
        Some(f) => f == name,
    }
}
