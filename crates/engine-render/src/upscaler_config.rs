//! Owned reader for the `[upscaler]` section of `engine.toml`.
//!
//! The schema (documented in `engine.toml`):
//!
//! ```toml
//! [upscaler]
//! provider = "auto" | "dlss" | "fsr" | "xess" | "owned-onnx" | "owned-bilinear"
//! quality  = "performance" | "balanced" | "quality" | "ultra-quality"
//! ```
//!
//! Mirrors the line-by-line pattern in
//! `bin/engine-bench-frame-pacing/src/budgets.rs`. No serde, no third-party
//! TOML parser — the schema is a flat key/value pair under a known
//! section header. Unknown keys + sections are silently skipped so an
//! older binary can read a newer file without aborting; invalid values
//! return [`ParseError`] so misconfiguration surfaces loudly at startup.

use core::fmt;
use std::path::Path;

/// Upscaler runtime provider selection.
///
/// Cascade order is fixed by ADR-066 §6 (DLSS → FSR → XeSS →
/// OwnedOnnxTemporal → OwnedBilinear); this enum names which slot the
/// operator wants the registry to *prefer* — `Auto` means the registry
/// walks the full cascade and picks the first one that
/// `supports(device)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider {
    /// Walk the cascade; pick the first supported provider.
    Auto,
    /// Force the NVIDIA DLSS provider; fall back to `OwnedBilinear`
    /// if `supports(device)` returns false at runtime.
    Dlss,
    /// Force the AMD FSR provider.
    Fsr,
    /// Force the Intel XeSS provider.
    Xess,
    /// Force the owned ONNX temporal upscaler (cross-vendor fallback).
    OwnedOnnx,
    /// Force the owned bilinear placeholder (RX-580 milestone target).
    OwnedBilinear,
}

impl Provider {
    /// Default per ADR-066: walk the cascade.
    pub const DEFAULT: Self = Self::Auto;
}

impl Default for Provider {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Internal-resolution scale factor selection.
///
/// Maps to the discrete render-target scales the upscaler chain
/// supports (ADR-005). Values mirror the `quality` knob the major
/// vendor SDKs expose so the same `engine.toml` works regardless of
/// the selected provider.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quality {
    /// 50% internal scale (highest performance, lowest quality).
    Performance,
    /// ~66% internal scale.
    Balanced,
    /// 75% internal scale.
    Quality,
    /// 100% internal scale (no upscaling).
    UltraQuality,
}

impl Quality {
    /// Default per ADR-005 §3.
    pub const DEFAULT: Self = Self::Balanced;
}

impl Default for Quality {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Parsed `[upscaler]` section.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UpscalerConfig {
    /// Operator-requested provider preference.
    pub provider: Provider,
    /// Operator-requested quality preset.
    pub quality: Quality,
}

/// Why an `engine.toml` could not be parsed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseError {
    /// `provider = "..."` value was not one of the documented variants.
    UnknownProvider(String),
    /// `quality = "..."` value was not one of the documented variants.
    UnknownQuality(String),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::UnknownProvider(s) => {
                write!(f, "unknown upscaler provider {s:?}")
            }
            ParseError::UnknownQuality(s) => {
                write!(f, "unknown upscaler quality {s:?}")
            }
        }
    }
}

impl std::error::Error for ParseError {}

/// Read + parse an `engine.toml` file from disk.
///
/// The reader scans only the `[upscaler]` section; the rest of the
/// manifest is silently ignored, so this helper can be called from
/// any consumer with a path to the top-level manifest.
pub fn read_from_path(path: &Path) -> Result<UpscalerConfig, String> {
    let body = std::fs::read_to_string(path).map_err(|e| format!("read {path:?}: {e}"))?;
    parse(&body).map_err(|e| format!("{path:?}: {e}"))
}

/// Parse an `engine.toml` body. Tolerates blank lines, `#` comments,
/// and arbitrary whitespace. Section headers other than `[upscaler]`
/// are ignored.
pub fn parse(source: &str) -> Result<UpscalerConfig, ParseError> {
    let mut config = UpscalerConfig::default();
    let mut in_section = false;
    for raw in source.lines() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            in_section = line == "[upscaler]";
            continue;
        }
        if !in_section {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = strip_comment(value).trim();
        let value = unquote(value);
        match key {
            "provider" => {
                config.provider = match value {
                    "auto" => Provider::Auto,
                    "dlss" => Provider::Dlss,
                    "fsr" => Provider::Fsr,
                    "xess" => Provider::Xess,
                    "owned-onnx" => Provider::OwnedOnnx,
                    "owned-bilinear" => Provider::OwnedBilinear,
                    other => return Err(ParseError::UnknownProvider(other.to_string())),
                };
            }
            "quality" => {
                config.quality = match value {
                    "performance" => Quality::Performance,
                    "balanced" => Quality::Balanced,
                    "quality" => Quality::Quality,
                    "ultra-quality" => Quality::UltraQuality,
                    other => return Err(ParseError::UnknownQuality(other.to_string())),
                };
            }
            _ => {}
        }
    }
    Ok(config)
}

fn strip_comment(line: &str) -> &str {
    match line.find('#') {
        Some(i) => &line[..i],
        None => line,
    }
}

/// Strip surrounding double or single quotes if present. The schema
/// quotes string values but the reader is lenient for one-off
/// hand-edited manifests.
fn unquote(s: &str) -> &str {
    let bytes = s.as_bytes();
    if bytes.len() >= 2
        && ((bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\''))
    {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_auto_balanced() {
        let cfg = UpscalerConfig::default();
        assert_eq!(cfg.provider, Provider::Auto);
        assert_eq!(cfg.quality, Quality::Balanced);
    }

    #[test]
    fn parse_canonical_section_round_trips() {
        let src = "\
[upscaler]
provider = \"auto\"
quality  = \"balanced\"
";
        let cfg = parse(src).expect("parses");
        assert_eq!(cfg.provider, Provider::Auto);
        assert_eq!(cfg.quality, Quality::Balanced);
    }

    #[test]
    fn parse_every_provider_variant() {
        for (raw, expected) in [
            ("auto", Provider::Auto),
            ("dlss", Provider::Dlss),
            ("fsr", Provider::Fsr),
            ("xess", Provider::Xess),
            ("owned-onnx", Provider::OwnedOnnx),
            ("owned-bilinear", Provider::OwnedBilinear),
        ] {
            let src = format!("[upscaler]\nprovider = \"{raw}\"\n");
            let cfg = parse(&src).expect("parses");
            assert_eq!(cfg.provider, expected, "raw {raw}");
        }
    }

    #[test]
    fn parse_every_quality_variant() {
        for (raw, expected) in [
            ("performance", Quality::Performance),
            ("balanced", Quality::Balanced),
            ("quality", Quality::Quality),
            ("ultra-quality", Quality::UltraQuality),
        ] {
            let src = format!("[upscaler]\nquality = \"{raw}\"\n");
            let cfg = parse(&src).expect("parses");
            assert_eq!(cfg.quality, expected, "raw {raw}");
        }
    }

    #[test]
    fn parse_rejects_unknown_provider() {
        let src = "[upscaler]\nprovider = \"radeon-rays\"\n";
        let err = parse(src).expect_err("should reject");
        assert!(matches!(err, ParseError::UnknownProvider(s) if s == "radeon-rays"));
    }

    #[test]
    fn parse_rejects_unknown_quality() {
        let src = "[upscaler]\nquality = \"insane\"\n";
        let err = parse(src).expect_err("should reject");
        assert!(matches!(err, ParseError::UnknownQuality(s) if s == "insane"));
    }

    #[test]
    fn parse_ignores_unrelated_sections() {
        let src = "[budgets]\nprovider = \"dlss\"\n";
        let cfg = parse(src).expect("parses");
        // [budgets] is ignored — defaults retained.
        assert_eq!(cfg.provider, Provider::Auto);
    }

    #[test]
    fn parse_tolerates_comments_and_unquoted_values() {
        let src = "\
# Phase 6 upscaler defaults.
[upscaler]
provider = dlss      # forced for the demo level
quality  = quality   # 75% scale
";
        let cfg = parse(src).expect("parses");
        assert_eq!(cfg.provider, Provider::Dlss);
        assert_eq!(cfg.quality, Quality::Quality);
    }

    #[test]
    fn parse_repo_engine_toml_is_default() {
        // The shipped engine.toml has the [upscaler] block commented
        // out (the schema-only example). The parser should return
        // defaults, not error.
        let body = include_str!("../../../engine.toml");
        let cfg = parse(body).expect("default config parses");
        assert_eq!(cfg.provider, Provider::Auto);
        assert_eq!(cfg.quality, Quality::Balanced);
    }
}
