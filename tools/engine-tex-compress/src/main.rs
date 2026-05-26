//! `engine-tex-compress` — BC{4,5,7} texture-import CLI (ADR-045 §4).
//!
//! Build-time tool. Reads a raw RGBA8 pixel buffer, runs the
//! `intel_tex_2` ISPC-based BC compressor, and writes a single output blob
//! shaped exactly as a pak entry will see it: an
//! [`engine_asset::TextureMeta`] header followed by the compressed BC bytes.
//!
//! Owned-discipline:
//! - No third-party argument parser (matches the pattern in
//!   `tools/engine-shader/`).
//! - No image-file decoders. The CLI consumes a raw RGBA8 stream so the
//!   choice of PNG/EXR decoder belongs to whoever feeds this tool.
//! - The compressor is a build-time dependency only; the engine runtime
//!   never links it (ADR-045 §"Consequences" line 111).
//!
//! Per ADR-045 §3 the engine ships complete mip chains baked at import time.
//! Mip-chain generation is **not** yet implemented in this PR — the CLI
//! refuses `--mips >1` for now and a follow-up CLI flag will land alongside
//! the importer hooked to Kaiser / sobel filters. PR 2 ships the codec path.
//!
//! ## Usage
//!
//! ```text
//! engine-tex-compress \
//!     --in <raw-rgba8.bin> \
//!     --out <output.tex> \
//!     --width <u32> \
//!     --height <u32> \
//!     --codec <bc4|bc5|bc7-srgb|bc7-linear> \
//!     --role <albedo|normal|rough-met-ao|hdr|ui>
//! ```
//!
//! Or to inspect a previously-built texture blob:
//!
//! ```text
//! engine-tex-compress --info <file.tex>
//! ```

use engine_asset::{ChannelRole, TEXTURE_META_BYTES, TexExtent, TexFormat, TextureMeta};
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    match parse(&argv) {
        Ok(Mode::Compress(opts)) => match compress(opts) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("engine-tex-compress: error: {e}");
                ExitCode::from(1)
            }
        },
        Ok(Mode::Info(path)) => match info(&path) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("engine-tex-compress: error: {e}");
                ExitCode::from(1)
            }
        },
        Ok(Mode::Help) => {
            print_help();
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("engine-tex-compress: {e}");
            eprintln!();
            print_help();
            ExitCode::from(2)
        }
    }
}

enum Mode {
    Compress(CompressOpts),
    Info(PathBuf),
    Help,
}

struct CompressOpts {
    input: PathBuf,
    output: PathBuf,
    width: u32,
    height: u32,
    codec: Codec,
    role: ChannelRole,
}

#[derive(Clone, Copy)]
enum Codec {
    Bc4,
    Bc5,
    Bc7Srgb,
    Bc7Linear,
}

impl Codec {
    fn to_format(self) -> TexFormat {
        match self {
            Codec::Bc4 => TexFormat::Bc4RUnorm,
            Codec::Bc5 => TexFormat::Bc5RgUnorm,
            Codec::Bc7Srgb => TexFormat::Bc7RgbaUnormSrgb,
            Codec::Bc7Linear => TexFormat::Bc7RgbaUnorm,
        }
    }
}

fn parse(argv: &[String]) -> Result<Mode, String> {
    if argv.is_empty() || matches!(argv[0].as_str(), "-h" | "--help") {
        return Ok(Mode::Help);
    }
    if argv[0] == "--info" {
        let path = argv
            .get(1)
            .ok_or_else(|| "--info needs a path".to_string())?;
        return Ok(Mode::Info(PathBuf::from(path)));
    }

    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut width: Option<u32> = None;
    let mut height: Option<u32> = None;
    let mut codec: Option<Codec> = None;
    let mut role: Option<ChannelRole> = None;
    let mut mips: Option<u8> = None;

    let mut i = 0;
    while i < argv.len() {
        let key = argv[i].as_str();
        let val = argv
            .get(i + 1)
            .ok_or_else(|| format!("flag {key} needs a value"))?;
        match key {
            "--in" => input = Some(PathBuf::from(val)),
            "--out" => output = Some(PathBuf::from(val)),
            "--width" => width = Some(val.parse().map_err(|_| "--width must be u32".to_string())?),
            "--height" => {
                height = Some(
                    val.parse()
                        .map_err(|_| "--height must be u32".to_string())?,
                )
            }
            "--codec" => {
                codec = Some(match val.as_str() {
                    "bc4" => Codec::Bc4,
                    "bc5" => Codec::Bc5,
                    "bc7-srgb" => Codec::Bc7Srgb,
                    "bc7-linear" => Codec::Bc7Linear,
                    other => return Err(format!("unknown --codec {other}")),
                });
            }
            "--role" => {
                role = Some(match val.as_str() {
                    "albedo" => ChannelRole::Albedo,
                    "normal" => ChannelRole::Normal,
                    "rough-met-ao" => ChannelRole::RoughMetAo,
                    "hdr" => ChannelRole::Hdr,
                    "ui" => ChannelRole::Ui,
                    other => return Err(format!("unknown --role {other}")),
                });
            }
            "--mips" => mips = Some(val.parse().map_err(|_| "--mips must be u8".to_string())?),
            other => return Err(format!("unknown flag {other}")),
        }
        i += 2;
    }

    if let Some(m) = mips
        && m != 1
    {
        return Err(
            "--mips >1 not yet supported (ADR-045 §3 mip-chain generation \
            lands in a follow-up)"
                .to_string(),
        );
    }

    Ok(Mode::Compress(CompressOpts {
        input: input.ok_or_else(|| "--in is required".to_string())?,
        output: output.ok_or_else(|| "--out is required".to_string())?,
        width: width.ok_or_else(|| "--width is required".to_string())?,
        height: height.ok_or_else(|| "--height is required".to_string())?,
        codec: codec.ok_or_else(|| "--codec is required".to_string())?,
        role: role.ok_or_else(|| "--role is required".to_string())?,
    }))
}

fn compress(opts: CompressOpts) -> Result<(), String> {
    if opts.width == 0 || opts.height == 0 {
        return Err("width and height must be non-zero".into());
    }
    // BC codecs operate on 4×4 blocks; non-block-multiples would require
    // padding which is a Phase-5-PR-3 importer concern. Reject for now.
    if !opts.width.is_multiple_of(4) || !opts.height.is_multiple_of(4) {
        return Err(format!(
            "width ({}) and height ({}) must be multiples of 4 (BC block size)",
            opts.width, opts.height
        ));
    }

    let rgba = std::fs::read(&opts.input).map_err(|e| format!("reading --in: {e}"))?;
    let expected = (opts.width as usize) * (opts.height as usize) * 4;
    if rgba.len() != expected {
        return Err(format!(
            "input size {} does not match width×height×4 = {}",
            rgba.len(),
            expected
        ));
    }

    let stride = opts.width * 4;
    let compressed = match opts.codec {
        Codec::Bc7Srgb | Codec::Bc7Linear => {
            let settings = intel_tex_2::bc7::opaque_basic_settings();
            let surface = intel_tex_2::RgbaSurface {
                data: &rgba,
                width: opts.width,
                height: opts.height,
                stride,
            };
            intel_tex_2::bc7::compress_blocks(&settings, &surface)
        }
        Codec::Bc5 => {
            let rg = extract_rg(&rgba);
            let surface = intel_tex_2::RgSurface {
                data: &rg,
                width: opts.width,
                height: opts.height,
                stride: opts.width * 2,
            };
            intel_tex_2::bc5::compress_blocks(&surface)
        }
        Codec::Bc4 => {
            let r = extract_r(&rgba);
            let surface = intel_tex_2::RSurface {
                data: &r,
                width: opts.width,
                height: opts.height,
                stride: opts.width,
            };
            intel_tex_2::bc4::compress_blocks(&surface)
        }
    };

    let meta = TextureMeta {
        format: opts.codec.to_format(),
        extent: TexExtent {
            width: opts.width,
            height: opts.height,
            layers: 1,
        },
        mip_count: 1,
        channel_role: opts.role,
    };
    let header = meta.encode(compressed.len() as u32);

    let mut out = Vec::with_capacity(TEXTURE_META_BYTES + compressed.len());
    out.extend_from_slice(&header);
    out.extend_from_slice(&compressed);
    std::fs::write(&opts.output, &out).map_err(|e| format!("writing --out: {e}"))?;

    eprintln!(
        "engine-tex-compress: {} {}×{} → {} bytes ({:?})",
        opts.input.display(),
        opts.width,
        opts.height,
        out.len(),
        opts.codec.to_format()
    );
    Ok(())
}

fn info(path: &PathBuf) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    let (meta, payload) =
        TextureMeta::decode(&bytes).map_err(|e| format!("decoding {}: {e:?}", path.display()))?;
    println!("file:        {}", path.display());
    println!("format:      {:?}", meta.format);
    println!("channel:     {:?}", meta.channel_role);
    println!(
        "extent:      {}×{}×{}",
        meta.extent.width, meta.extent.height, meta.extent.layers
    );
    println!("mip_count:   {}", meta.mip_count);
    println!("payload:     {} bytes", payload.len());
    println!("total:       {} bytes", bytes.len());
    Ok(())
}

fn extract_rg(rgba: &[u8]) -> Vec<u8> {
    rgba.chunks_exact(4).flat_map(|c| [c[0], c[1]]).collect()
}

fn extract_r(rgba: &[u8]) -> Vec<u8> {
    rgba.iter().step_by(4).copied().collect()
}

fn print_help() {
    eprintln!(
        "engine-tex-compress — BC{{4,5,7}} import CLI (ADR-045 §4)\n\
         \n\
         Usage:\n\
           engine-tex-compress --in <rgba8> --out <tex> --width <u32> --height <u32> \\\n\
                               --codec <bc4|bc5|bc7-srgb|bc7-linear> \\\n\
                               --role <albedo|normal|rough-met-ao|hdr|ui>\n\
         \n\
           engine-tex-compress --info <tex-file>\n\
         \n\
         Input is a raw RGBA8 pixel buffer (no PNG decoding here).\n\
         Output is the bytes a pak entry will contain:\n\
           [TextureMeta header || compressed BC payload]\n"
    );
}
