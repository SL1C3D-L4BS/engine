//! `engine-bench-frame-pacing` — frame-pacing milestone bench binary
//! (Phase 5 PR 5, ADR-005 + ADR-047).
//!
//! Runs the Phase-5 standard scenario at 1440p with the RX-580 quality
//! preset, captures per-frame wall-clock, and emits a JSON report.
//!
//! Two modes:
//!
//! - `--run` — execute the scenario, write the report (defaults to
//!   stdout; `--output <path>` redirects). PR-5 informational: the
//!   report is recorded in `docs/observatory/phase-5-milestone-
//!   baseline.md`. The same binary runs in PR 6 as the formal CI gate.
//! - `--gate <report.json>` — read a previously-written report and
//!   evaluate it against the budgets file. PR-5 informational
//!   (`continue-on-error: true` per ADR-047 §7); transitions to a
//!   required check in PR 6.
//!
//! ## Owned discipline
//!
//! - Own argument parser (no clap).
//! - Own JSON writer + minimal reader (no serde, no serde_json — same
//!   pattern as `tools/sampling-profiler/`, `tools/engine-shader/`).
//! - The PR-5 scenario is a deterministic CPU workload: a synthetic
//!   HDR gradient is bilinearly upscaled from 1280×720 to 2560×1440
//!   per frame, exactly the trait-surface integration ADR-005 calls
//!   for. The frame-time signal is the upscaler placeholder's cost +
//!   the surrounding allocator + per-frame setup. GPU-backed numbers
//!   land when the self-hosted RX 6700 XT runner stands up in PR 6
//!   (ADR-047 §2).

mod bench;
mod budgets;
mod json;
mod scene;
mod stats;

use std::path::PathBuf;
use std::process::ExitCode;

use bench::{Scenario, ScenarioReport, run_scenario};
use stats::{p99_ms, stddev_ms};

/// ADR-047 §3 thresholds. PR 5 inherits these as informational
/// reference values; PR 6 promotes the gate to required.
const SPEC_P99_MS: f64 = 18.3;
/// ADR-047 §3 standard-deviation budget at 60 FPS.
const SPEC_STDDEV_MS: f64 = 1.04;

#[derive(Debug)]
enum Mode {
    Run(RunOpts),
    Gate(GateOpts),
    Help,
}

#[derive(Debug)]
struct RunOpts {
    frames: u32,
    input_extent: [u32; 2],
    output_extent: [u32; 2],
    output: Option<PathBuf>,
    /// Set when `--scene PATH` is given. The bench loads + hashes +
    /// parses the file; the parsed `Scene` overrides extents and frame
    /// count, and the scene hash is included in the JSON report.
    scene: Option<SceneInputs>,
}

#[derive(Debug)]
struct SceneInputs {
    path: PathBuf,
    hash_hex: String,
    parsed: scene::Scene,
}

impl Default for RunOpts {
    fn default() -> Self {
        Self {
            frames: 60,
            input_extent: [
                engine_raster::MILESTONE_INPUT_WIDTH,
                engine_raster::MILESTONE_INPUT_HEIGHT,
            ],
            output_extent: [
                engine_raster::MILESTONE_OUTPUT_WIDTH,
                engine_raster::MILESTONE_OUTPUT_HEIGHT,
            ],
            output: None,
            scene: None,
        }
    }
}

#[derive(Debug)]
struct GateOpts {
    report: PathBuf,
    p99_budget_ms: f64,
    stddev_budget_ms: f64,
    /// `Some` only when `--budgets PATH` was supplied (or in tests). The
    /// path is recorded so the verdict message can name it.
    budgets_path: Option<PathBuf>,
}

const USAGE: &str = "\
engine-bench-frame-pacing — Phase-5 milestone + ADR-047 frame-pacing bench

USAGE:
    engine-bench-frame-pacing --run [--scene PATH] [--frames N] [--input WxH] [--output WxH] [--output-path PATH]
    engine-bench-frame-pacing --gate <REPORT.json> [--budgets PATH] [--p99 MS] [--stddev MS]
    engine-bench-frame-pacing --help

RUN OPTIONS:
    --scene PATH         Load the deterministic scene fixture (RON, ADR-047 §1).
                         Sets frames + extents from the file. --frames /
                         --input / --output override per-flag if also given.
                         The scene's BLAKE3 hash is recorded in the report.
    --frames N           Frames to measure (default 60).
    --input WxH          Internal render resolution (default 1280x720).
    --output WxH         Display resolution (default 2560x1440).
    --output-path PATH   Write the JSON report to this file (default stdout).

GATE OPTIONS:
    <REPORT.json>        Required positional — the JSON report produced by --run.
    --budgets PATH       Read p99 / stddev thresholds from a TOML file
                         (canonical: tools/frame-pacing/budgets.toml).
                         --p99 / --stddev override per-field if also given.
    --p99 MS             p99 frame-time budget in milliseconds (default 18.3, ADR-047 §3).
    --stddev MS          Stddev frame-time budget in milliseconds (default 1.04, ADR-047 §3).

EXIT STATUS:
    0  Success (or PR-5 informational PASS / FAIL — gate never fails the
       build until PR 6 lands; see ADR-047 §7).
    1  I/O or scenario error.
    2  Argument-parse error.
";

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mode = match parse_args(&argv) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("engine-bench-frame-pacing: {e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match mode {
        Mode::Help => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        Mode::Run(opts) => match run(opts) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("engine-bench-frame-pacing: run failed: {e}");
                ExitCode::from(1)
            }
        },
        Mode::Gate(opts) => match gate(opts) {
            Ok(verdict) => {
                println!("{verdict}");
                // PR-5 mode: never fail the build. ADR-047 §7.
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("engine-bench-frame-pacing: gate failed: {e}");
                ExitCode::from(1)
            }
        },
    }
}

fn parse_args(argv: &[String]) -> Result<Mode, String> {
    let mut mode_flag: Option<&str> = None;
    let mut run_opts = RunOpts::default();
    let mut gate_report: Option<PathBuf> = None;
    let mut gate_p99_override: Option<f64> = None;
    let mut gate_stddev_override: Option<f64> = None;
    let mut budgets_path: Option<PathBuf> = None;
    let mut i = 0;
    while i < argv.len() {
        let a = &argv[i];
        match a.as_str() {
            "-h" | "--help" => return Ok(Mode::Help),
            "--run" => mode_flag = Some("run"),
            "--gate" => {
                mode_flag = Some("gate");
                let v = argv
                    .get(i + 1)
                    .ok_or_else(|| "--gate requires a path argument".to_string())?;
                gate_report = Some(PathBuf::from(v));
                i += 1;
            }
            "--budgets" => {
                let v = argv
                    .get(i + 1)
                    .ok_or_else(|| "--budgets requires a path argument".to_string())?;
                budgets_path = Some(PathBuf::from(v));
                i += 1;
            }
            "--scene" => {
                let v = argv
                    .get(i + 1)
                    .ok_or_else(|| "--scene requires a path argument".to_string())?;
                let path = PathBuf::from(v);
                let body = std::fs::read_to_string(&path)
                    .map_err(|e| format!("--scene: read {path:?}: {e}"))?;
                let parsed = scene::parse(&body)
                    .map_err(|e| format!("--scene: parse {path:?}: {e}"))?;
                let hash_hex = scene::scene_hash_hex(body.as_bytes());
                run_opts.frames = parsed.frames;
                run_opts.input_extent = parsed.internal_extent;
                run_opts.output_extent = parsed.output_extent;
                run_opts.scene = Some(SceneInputs {
                    path,
                    hash_hex,
                    parsed,
                });
                i += 1;
            }
            "--frames" => {
                let v = argv
                    .get(i + 1)
                    .ok_or_else(|| "--frames requires a value".to_string())?;
                run_opts.frames = v
                    .parse()
                    .map_err(|_| format!("bad --frames value: {v}"))?;
                i += 1;
            }
            "--input" => {
                let v = argv
                    .get(i + 1)
                    .ok_or_else(|| "--input requires WxH".to_string())?;
                run_opts.input_extent = parse_extent(v)?;
                i += 1;
            }
            "--output" => {
                let v = argv
                    .get(i + 1)
                    .ok_or_else(|| "--output requires WxH".to_string())?;
                run_opts.output_extent = parse_extent(v)?;
                i += 1;
            }
            "--output-path" => {
                let v = argv
                    .get(i + 1)
                    .ok_or_else(|| "--output-path requires a path".to_string())?;
                run_opts.output = Some(PathBuf::from(v));
                i += 1;
            }
            "--p99" => {
                let v = argv
                    .get(i + 1)
                    .ok_or_else(|| "--p99 requires a value".to_string())?;
                gate_p99_override =
                    Some(v.parse().map_err(|_| format!("bad --p99 value: {v}"))?);
                i += 1;
            }
            "--stddev" => {
                let v = argv
                    .get(i + 1)
                    .ok_or_else(|| "--stddev requires a value".to_string())?;
                gate_stddev_override = Some(
                    v.parse()
                        .map_err(|_| format!("bad --stddev value: {v}"))?,
                );
                i += 1;
            }
            other => return Err(format!("unknown argument: {other}")),
        }
        i += 1;
    }
    match mode_flag {
        Some("run") => {
            if budgets_path.is_some() {
                return Err("--budgets is only valid with --gate".into());
            }
            Ok(Mode::Run(run_opts))
        }
        Some("gate") if run_opts.scene.is_some() => {
            Err("--scene is only valid with --run".into())
        }
        Some("gate") => {
            // Resolution: file < explicit flag < spec default. Start at
            // the spec defaults; overlay the file if present; overlay
            // the per-flag overrides last.
            let mut p99 = SPEC_P99_MS;
            let mut stddev = SPEC_STDDEV_MS;
            if let Some(path) = budgets_path.as_deref() {
                let b = budgets::read_from_path(path)?;
                if let Some(v) = b.p99_ms {
                    p99 = v;
                }
                if let Some(v) = b.stddev_ms {
                    stddev = v;
                }
            }
            if let Some(v) = gate_p99_override {
                p99 = v;
            }
            if let Some(v) = gate_stddev_override {
                stddev = v;
            }
            Ok(Mode::Gate(GateOpts {
                report: gate_report.expect("gate path captured above"),
                p99_budget_ms: p99,
                stddev_budget_ms: stddev,
                budgets_path,
            }))
        }
        Some(_) => unreachable!("mode_flag set to a known value only"),
        None => Err("no mode specified — pass --run or --gate".into()),
    }
}

fn parse_extent(s: &str) -> Result<[u32; 2], String> {
    let (w, h) = s
        .split_once('x')
        .ok_or_else(|| format!("extent must be WxH (e.g. 1280x720); got: {s}"))?;
    let w: u32 = w.parse().map_err(|_| format!("bad width: {w}"))?;
    let h: u32 = h.parse().map_err(|_| format!("bad height: {h}"))?;
    if w == 0 || h == 0 {
        return Err(format!("extent must be positive; got: {s}"));
    }
    Ok([w, h])
}

fn run(opts: RunOpts) -> Result<(), String> {
    let scenario = Scenario {
        frames: opts.frames,
        input_extent: opts.input_extent,
        output_extent: opts.output_extent,
    };
    let report = run_scenario(&scenario).map_err(|e| e.to_string())?;
    let json = serialize_report(&report, opts.scene.as_ref());
    match opts.output {
        Some(path) => std::fs::write(&path, &json).map_err(|e| format!("write {path:?}: {e}"))?,
        None => println!("{json}"),
    }
    Ok(())
}

fn gate(opts: GateOpts) -> Result<String, String> {
    let body = std::fs::read_to_string(&opts.report)
        .map_err(|e| format!("read {:?}: {e}", opts.report))?;
    let p99 =
        json::read_top_level_number(&body, "p99_ms").ok_or("report missing `p99_ms` field")?;
    let stddev = json::read_top_level_number(&body, "stddev_ms")
        .ok_or("report missing `stddev_ms` field")?;
    let p99_ok = p99 <= opts.p99_budget_ms;
    let stddev_ok = stddev <= opts.stddev_budget_ms;
    let verdict = if p99_ok && stddev_ok { "PASS" } else { "FAIL" };
    let source = match opts.budgets_path.as_deref() {
        Some(p) => format!("budgets: {}", p.display()),
        None => "budgets: spec defaults (ADR-047 §3)".to_string(),
    };
    Ok(format!(
        "engine-bench-frame-pacing gate: {verdict} \
         (p99 {p99:.3} ms / budget {:.3}, stddev {stddev:.3} ms / budget {:.3}; {source}) \
         [PR-5 informational; gate activates in PR 6 per ADR-047 §7]",
        opts.p99_budget_ms, opts.stddev_budget_ms
    ))
}

fn serialize_report(report: &ScenarioReport, scene: Option<&SceneInputs>) -> String {
    let mut w = json::JsonWriter::new();
    w.begin_object();
    w.field_str("scenario", "phase-5-milestone-cpu-oracle");
    w.field_str("adr", "ADR-005 + ADR-047");
    w.field_str("upscaler", "owned.bilinear");
    if let Some(s) = scene {
        w.field_str("scene_path", &s.path.to_string_lossy());
        w.field_str("scene_hash", &s.hash_hex);
        w.field_str("scene_quality", &s.parsed.quality);
        w.field_u64("scene_target_fps", s.parsed.target_fps as u64);
    }
    w.field_u64("frames", report.frame_times_ns.len() as u64);
    w.field_array_u32_2("input_extent", report.input_extent);
    w.field_array_u32_2("output_extent", report.output_extent);
    w.field_f64("mean_ms", report.mean_ms);
    w.field_f64("p99_ms", p99_ms(&report.frame_times_ns));
    w.field_f64("stddev_ms", stddev_ms(&report.frame_times_ns));
    w.field_f64("min_ms", report.min_ms);
    w.field_f64("max_ms", report.max_ms);
    w.field_array_f64(
        "frame_time_ms",
        report
            .frame_times_ns
            .iter()
            .map(|ns| (*ns as f64) / 1_000_000.0)
            .collect::<Vec<_>>(),
    );
    w.end_object();
    w.into_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extent_accepts_well_formed_pair() {
        assert_eq!(parse_extent("1280x720").unwrap(), [1280, 720]);
        assert_eq!(parse_extent("2560x1440").unwrap(), [2560, 1440]);
    }

    #[test]
    fn parse_extent_rejects_zero_and_garbage() {
        assert!(parse_extent("0x720").is_err());
        assert!(parse_extent("1280x0").is_err());
        assert!(parse_extent("hello").is_err());
        assert!(parse_extent("1280-720").is_err());
    }

    #[test]
    fn parse_args_help_short_and_long() {
        let m = parse_args(&["-h".into()]).unwrap();
        assert!(matches!(m, Mode::Help));
        let m = parse_args(&["--help".into()]).unwrap();
        assert!(matches!(m, Mode::Help));
    }

    #[test]
    fn parse_args_run_defaults_match_milestone_constants() {
        let m = parse_args(&["--run".into()]).unwrap();
        let Mode::Run(o) = m else {
            panic!("expected Run mode")
        };
        assert_eq!(o.frames, 60);
        assert_eq!(
            o.input_extent,
            [
                engine_raster::MILESTONE_INPUT_WIDTH,
                engine_raster::MILESTONE_INPUT_HEIGHT
            ]
        );
        assert_eq!(
            o.output_extent,
            [
                engine_raster::MILESTONE_OUTPUT_WIDTH,
                engine_raster::MILESTONE_OUTPUT_HEIGHT
            ]
        );
        assert!(o.output.is_none());
    }

    #[test]
    fn parse_args_run_with_explicit_overrides() {
        let m = parse_args(&[
            "--run".into(),
            "--frames".into(),
            "12".into(),
            "--input".into(),
            "640x360".into(),
            "--output".into(),
            "1280x720".into(),
            "--output-path".into(),
            "/tmp/out.json".into(),
        ])
        .unwrap();
        let Mode::Run(o) = m else {
            panic!("expected Run")
        };
        assert_eq!(o.frames, 12);
        assert_eq!(o.input_extent, [640, 360]);
        assert_eq!(o.output_extent, [1280, 720]);
        assert_eq!(o.output, Some(PathBuf::from("/tmp/out.json")));
    }

    #[test]
    fn parse_args_gate_requires_a_path() {
        let err = parse_args(&["--gate".into()]).unwrap_err();
        assert!(err.contains("--gate"));
    }

    #[test]
    fn parse_args_gate_with_default_budgets() {
        let m = parse_args(&["--gate".into(), "report.json".into()]).unwrap();
        let Mode::Gate(g) = m else {
            panic!("expected Gate")
        };
        assert_eq!(g.report, PathBuf::from("report.json"));
        assert!((g.p99_budget_ms - SPEC_P99_MS).abs() < 1e-9);
        assert!((g.stddev_budget_ms - SPEC_STDDEV_MS).abs() < 1e-9);
    }

    #[test]
    fn parse_args_no_mode_is_an_error() {
        let err = parse_args(&[]).unwrap_err();
        assert!(err.contains("no mode"));
    }

    #[test]
    fn parse_args_gate_with_budgets_file_overrides_defaults() {
        // Write a budgets file to a temp path with non-default values.
        let dir = std::env::temp_dir();
        let path = dir.join("engine-bench-frame-pacing-test-budgets.toml");
        std::fs::write(
            &path,
            "[budgets]\np99_ms = 12.5\nstddev_ms = 0.75\n",
        )
        .unwrap();
        let m = parse_args(&[
            "--gate".into(),
            "report.json".into(),
            "--budgets".into(),
            path.to_string_lossy().into_owned(),
        ])
        .unwrap();
        let Mode::Gate(g) = m else {
            panic!("expected Gate")
        };
        assert!((g.p99_budget_ms - 12.5).abs() < 1e-9);
        assert!((g.stddev_budget_ms - 0.75).abs() < 1e-9);
        assert_eq!(g.budgets_path.as_deref(), Some(path.as_path()));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn parse_args_gate_explicit_flags_override_budgets_file() {
        // CLI flags win over the file's contents (file < explicit flag).
        let dir = std::env::temp_dir();
        let path = dir.join("engine-bench-frame-pacing-test-budgets-2.toml");
        std::fs::write(
            &path,
            "[budgets]\np99_ms = 12.5\nstddev_ms = 0.75\n",
        )
        .unwrap();
        let m = parse_args(&[
            "--gate".into(),
            "report.json".into(),
            "--budgets".into(),
            path.to_string_lossy().into_owned(),
            "--p99".into(),
            "20.0".into(),
        ])
        .unwrap();
        let Mode::Gate(g) = m else {
            panic!("expected Gate")
        };
        assert!((g.p99_budget_ms - 20.0).abs() < 1e-9);
        // stddev still comes from the file.
        assert!((g.stddev_budget_ms - 0.75).abs() < 1e-9);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn parse_args_budgets_with_run_is_an_error() {
        let err = parse_args(&[
            "--run".into(),
            "--budgets".into(),
            "/nonexistent/path".into(),
        ])
        .unwrap_err();
        assert!(err.contains("--budgets"));
    }

    #[test]
    fn serialize_report_round_trips_p99_and_stddev() {
        let report = ScenarioReport {
            frame_times_ns: vec![1_000_000, 2_000_000, 3_000_000, 4_000_000],
            input_extent: [1280, 720],
            output_extent: [2560, 1440],
            mean_ms: 2.5,
            min_ms: 1.0,
            max_ms: 4.0,
        };
        let s = serialize_report(&report, None);
        let p99 = json::read_top_level_number(&s, "p99_ms").unwrap();
        let stddev = json::read_top_level_number(&s, "stddev_ms").unwrap();
        // p99 of [1, 2, 3, 4] ms = 4 ms (highest sample at the ≥99%
        // percentile — small-N edge case).
        assert!((p99 - 4.0).abs() < 1e-6);
        // stddev of [1, 2, 3, 4] (population) = sqrt(((-1.5)² + (-0.5)² +
        // (0.5)² + (1.5)²) / 4) = sqrt(1.25) ≈ 1.118
        assert!((stddev - 1.118).abs() < 0.01);
        // Report shape: starts/ends with braces (top-level object).
        assert!(s.trim_start().starts_with('{'));
        assert!(s.trim_end().ends_with('}'));
        // Contains the canonical scenario tag.
        assert!(s.contains("\"scenario\":\"phase-5-milestone-cpu-oracle\""));
        assert!(s.contains("\"upscaler\":\"owned.bilinear\""));
        // No scene fields when no scene was loaded.
        assert!(!s.contains("\"scene_hash\""));
    }

    #[test]
    fn serialize_report_with_scene_emits_hash_and_path() {
        let report = ScenarioReport {
            frame_times_ns: vec![1_000_000, 2_000_000],
            input_extent: [1280, 720],
            output_extent: [2560, 1440],
            mean_ms: 1.5,
            min_ms: 1.0,
            max_ms: 2.0,
        };
        let scene_inputs = SceneInputs {
            path: PathBuf::from("/tmp/test-scene.ron"),
            hash_hex: "abc123".to_string(),
            parsed: scene::Scene {
                seed: 0,
                frames: 3600,
                entities: 10000,
                unique_meshes: 50,
                directional_lights: 16,
                point_spot_lights: 48,
                camera_seed: 0,
                quality: "rx-580".to_string(),
                target_fps: 60,
                internal_extent: [1280, 720],
                output_extent: [2560, 1440],
            },
        };
        let s = serialize_report(&report, Some(&scene_inputs));
        assert!(s.contains("\"scene_path\":\"/tmp/test-scene.ron\""));
        assert!(s.contains("\"scene_hash\":\"abc123\""));
        assert!(s.contains("\"scene_quality\":\"rx-580\""));
        assert!(s.contains("\"scene_target_fps\":60"));
    }

    #[test]
    fn parse_args_run_with_canonical_scene_loads_extents() {
        let canonical = "testbed/frame-pacing/scenes/v0.ron";
        // Cargo runs tests from the workspace root.
        if !std::path::Path::new(canonical).exists() {
            // Not running from workspace root — skip without panicking.
            return;
        }
        let m = parse_args(&["--run".into(), "--scene".into(), canonical.into()]).unwrap();
        let Mode::Run(o) = m else {
            panic!("expected Run")
        };
        // Scene parameters override defaults.
        assert_eq!(o.frames, 3600);
        assert_eq!(o.input_extent, [1280, 720]);
        assert_eq!(o.output_extent, [2560, 1440]);
        let scene = o.scene.expect("scene loaded");
        assert_eq!(scene.path, PathBuf::from(canonical));
        assert_eq!(scene.hash_hex.len(), 64);
        assert_eq!(scene.parsed.quality, "rx-580");
    }

    #[test]
    fn parse_args_scene_with_gate_is_an_error() {
        // Write a minimal v0.ron to a temp path.
        let dir = std::env::temp_dir();
        let path = dir.join("engine-bench-test-scene-gate-error.ron");
        std::fs::write(
            &path,
            r#"FramePacingScene(
    seed: 0,
    frames: 60,
    entities: 1,
    unique_meshes: 1,
    directional_lights: 0,
    point_spot_lights: 0,
    camera_seed: 0,
    quality: "test",
    target_fps: 60,
    internal_extent: (640, 360),
    output_extent: (1280, 720),
)"#,
        )
        .unwrap();
        let err = parse_args(&[
            "--gate".into(),
            "report.json".into(),
            "--scene".into(),
            path.to_string_lossy().into_owned(),
        ])
        .unwrap_err();
        assert!(err.contains("--scene"));
        std::fs::remove_file(&path).ok();
    }
}
