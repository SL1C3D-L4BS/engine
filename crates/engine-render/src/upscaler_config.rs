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
//! Phase 6 PR 1d (ADR-082): the line-iteration + section-walk +
//! comment-stripping + quote-awareness layer lives in the shared
//! [`engine_config`] crate. This module names only the two domain
//! enums (`Provider`, `Quality`) the schema carries; invalid values
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
    /// 67% internal scale (matches the `engine.toml [upscaler]` schema).
    Balanced,
    /// 75% internal scale.
    Quality,
    /// 100% internal scale (no upscaling).
    UltraQuality,
}

impl Quality {
    /// Default per ADR-005 §3.
    pub const DEFAULT: Self = Self::Balanced;

    /// Internal-resolution divisor: `floor(display_extent * scale())`
    /// produces the upscaler's input extent for this quality preset.
    /// Values match the documented `engine.toml [upscaler]` schema.
    pub fn scale(self) -> f32 {
        match self {
            Quality::Performance => 0.50,
            Quality::Balanced => 0.67,
            Quality::Quality => 0.75,
            Quality::UltraQuality => 1.00,
        }
    }
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
    /// A quoted value was missing its closing quote (e.g. `provider = "dlss`).
    /// The schema specifies quoted strings; an unbalanced quote is a hard
    /// parse error rather than a silent pass-through that produces a
    /// confusing UnknownProvider/UnknownQuality with a stray leading `"`.
    UnbalancedQuote {
        /// The malformed value verbatim.
        value: String,
    },
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
            ParseError::UnbalancedQuote { value } => {
                write!(f, "unbalanced quote in value {value:?}")
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
///
/// Phase 6 PR 1d (ADR-082): delegates the line-iteration + section
/// walk + comment-stripping + quote-awareness to [`engine_config`];
/// this body only names the two domain enums.
pub fn parse(source: &str) -> Result<UpscalerConfig, ParseError> {
    let mut config = UpscalerConfig::default();
    let cfg = engine_config::parse(source).map_err(|e| match e {
        engine_config::ParseError::UnterminatedString { line, .. } => {
            // Reconstruct the malformed value from the source for the
            // `UnbalancedQuote { value }` carry. Lines are 1-based.
            let value = source
                .lines()
                .nth(line.saturating_sub(1))
                .and_then(|raw| raw.split_once('=').map(|(_, v)| v.trim()))
                .unwrap_or("")
                .to_string();
            ParseError::UnbalancedQuote { value }
        }
        // Any other malformation surfaces as an unbalanced-quote-like
        // structural error — the schema does not exercise the other
        // engine_config variants (no empty keys, no malformed headers
        // in `[upscaler]`'s flat schema).
        _ => ParseError::UnbalancedQuote {
            value: String::new(),
        },
    })?;

    // Walk every `[upscaler]` section (last assignment wins per the
    // engine_config semantics). Section names are matched verbatim;
    // engine_config already trims internal whitespace inside the
    // brackets, so `[ upscaler ]` and `[upscaler]` both land here.
    let Some(section) = cfg.section("upscaler") else {
        return Ok(config);
    };
    for (key, value) in &section.entries {
        let Some(raw) = value.as_str() else {
            // Numeric / bool values for these keys aren't in the schema;
            // skip silently for forward-compat.
            continue;
        };
        match key.as_str() {
            "provider" => {
                config.provider = match raw {
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
                config.quality = match raw {
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
    fn strip_comment_respects_quoted_hash() {
        // `#` inside a quoted value must not be treated as a comment
        // start. The value "with#hash" round-trips through strip_comment
        // and unquote; the parser then rejects it as an unknown provider
        // (which proves the `#` was preserved through the read).
        let src = "[upscaler]\nprovider = \"with#hash\"\n";
        let err = parse(src).expect_err("not a real provider");
        assert!(
            matches!(err, ParseError::UnknownProvider(ref s) if s == "with#hash"),
            "expected UnknownProvider(\"with#hash\"), got {err:?}",
        );
    }

    #[test]
    fn parse_rejects_unbalanced_quote() {
        // Missing closing quote — the schema mandates quoted values, so
        // this is a hard parse error rather than a silent pass-through.
        let src = "[upscaler]\nprovider = \"dlss\n";
        let err = parse(src).expect_err("should reject");
        assert!(
            matches!(err, ParseError::UnbalancedQuote { ref value } if value == "\"dlss"),
            "got {err:?}",
        );
    }

    #[test]
    fn parse_section_header_tolerates_internal_whitespace() {
        // Idiomatic TOML accepts whitespace inside the brackets. The
        // parser should treat `[ upscaler ]` and `[upscaler]` identically.
        let src = "[ upscaler ]\nprovider = \"dlss\"\n";
        let cfg = parse(src).expect("parses");
        assert_eq!(cfg.provider, Provider::Dlss);
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
