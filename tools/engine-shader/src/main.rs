//! `engine-shader` CLI — compile a `.slang` source to one or all
//! engine targets (SPIR-V / WGSL / DXIL / MSL) and write a bundle.
//!
//! Owned arg parsing — no clap dependency (project rule R-02).
//!
//! Usage:
//!
//! ```text
//! engine-shader -i <source.slang> -e <entry> -s <vertex|fragment|compute>
//!               [-t <spirv|wgsl|dxil|msl|all>]
//!               -o <bundle.shdr>
//!               [--permissive]
//! ```

use engine_shader::artifact::Bundle;
use engine_shader::slangc::Compiler;
use engine_shader::target::{Stage, Target};
use engine_shader::{Artifact, compile_all_targets, encode};
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let opts = match parse_args(&args) {
        Ok(o) => o,
        Err(msg) => {
            eprintln!("error: {msg}\n");
            print_usage();
            return ExitCode::from(2);
        }
    };
    if opts.help {
        print_usage();
        return ExitCode::SUCCESS;
    }
    let compiler = match if opts.permissive {
        Compiler::permissive()
    } else {
        Compiler::locate()
    } {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(1);
        }
    };
    let bundle: Bundle = if let Some(target) = opts.target {
        match compiler.compile_with_reflection(&opts.source, &opts.entry, opts.stage, target, None)
        {
            Ok((bytes, refl)) => Bundle::new(
                &opts.entry,
                opts.stage,
                vec![Artifact::new(target, bytes, refl.unwrap_or_default())],
            ),
            Err(e) => {
                eprintln!("error: {e}");
                return ExitCode::from(1);
            }
        }
    } else {
        let (bundle, errors) =
            compile_all_targets(&compiler, &opts.source, &opts.entry, opts.stage);
        for e in &errors {
            eprintln!("warning: {e}");
        }
        if bundle.artifacts.is_empty() {
            eprintln!("error: all targets failed");
            return ExitCode::from(1);
        }
        bundle
    };
    let encoded = encode(&bundle);
    if let Err(e) = std::fs::write(&opts.output, &encoded) {
        eprintln!("error: writing {}: {e}", opts.output.display());
        return ExitCode::from(1);
    }
    eprintln!(
        "engine-shader: {} bytes, {} target(s), digest {}",
        encoded.len(),
        bundle.artifacts.len(),
        hex(bundle.bundle_digest())
    );
    ExitCode::SUCCESS
}

#[derive(Debug)]
struct Opts {
    source: PathBuf,
    entry: String,
    stage: Stage,
    target: Option<Target>,
    output: PathBuf,
    permissive: bool,
    help: bool,
}

fn parse_args(args: &[String]) -> Result<Opts, String> {
    let mut source: Option<PathBuf> = None;
    let mut entry: Option<String> = None;
    let mut stage: Option<Stage> = None;
    let mut target: Option<Target> = None;
    let mut all_targets = false;
    let mut output: Option<PathBuf> = None;
    let mut permissive = false;
    let mut help = false;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-i" | "--input" => {
                source = Some(PathBuf::from(it.next().ok_or("`-i` needs a path")?));
            }
            "-e" | "--entry" => {
                entry = Some(it.next().ok_or("`-e` needs a name")?.to_string());
            }
            "-s" | "--stage" => {
                stage = Some(parse_stage(it.next().ok_or("`-s` needs a stage")?)?);
            }
            "-t" | "--target" => {
                let v = it.next().ok_or("`-t` needs a target")?;
                if v == "all" {
                    all_targets = true;
                } else {
                    target = Some(parse_target(v)?);
                }
            }
            "-o" | "--output" => {
                output = Some(PathBuf::from(it.next().ok_or("`-o` needs a path")?));
            }
            "--permissive" => permissive = true,
            "-h" | "--help" => help = true,
            other => return Err(format!("unknown arg: {other}")),
        }
    }
    if help {
        return Ok(Opts {
            source: PathBuf::new(),
            entry: String::new(),
            stage: Stage::Vertex,
            target: None,
            output: PathBuf::new(),
            permissive,
            help,
        });
    }
    let source = source.ok_or("missing `-i <source>`")?;
    let entry = entry.ok_or("missing `-e <entry>`")?;
    let stage = stage.ok_or("missing `-s <stage>`")?;
    let output = output.ok_or("missing `-o <output>`")?;
    if all_targets {
        target = None;
    }
    Ok(Opts {
        source,
        entry,
        stage,
        target,
        output,
        permissive,
        help: false,
    })
}

fn parse_stage(s: &str) -> Result<Stage, String> {
    match s {
        "vertex" | "vs" => Ok(Stage::Vertex),
        "fragment" | "fs" | "pixel" | "ps" => Ok(Stage::Fragment),
        "compute" | "cs" => Ok(Stage::Compute),
        other => Err(format!("unknown stage: {other}")),
    }
}

fn parse_target(s: &str) -> Result<Target, String> {
    match s {
        "spirv" | "spv" => Ok(Target::SpirV),
        "wgsl" => Ok(Target::Wgsl),
        "dxil" => Ok(Target::Dxil),
        "msl" | "metal" => Ok(Target::Msl),
        other => Err(format!("unknown target: {other}")),
    }
}

fn print_usage() {
    eprintln!(
        "engine-shader — compile a .slang source via slangc (ADR-037)\n\
         \n\
         usage:\n  \
         engine-shader -i <source.slang> -e <entry> -s <stage> -o <bundle.shdr>\n  \
                       [-t <spirv|wgsl|dxil|msl|all>] [--permissive]\n\
         \n\
         stage: vertex | fragment | compute\n\
         target: spirv | wgsl | dxil | msl | all (default: all)\n\
         --permissive: do not enforce the SLANGC_PIN version match"
    );
}

fn hex(bytes: [u8; 32]) -> String {
    let mut out = String::with_capacity(64);
    for b in &bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}
