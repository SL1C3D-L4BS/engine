//! `engine-upscale-vendor` — per-vendor upscaler SDK bindings
//! (Phase 6 PR 5.5 scaffold per ADR-066 §1).
//!
//! Each supported vendor (NVIDIA DLSS / AMD FSR / Intel XeSS) and the
//! owned ONNX temporal upscaler live behind a cargo feature flag here.
//! Activating a feature pulls in the corresponding `*-sys` crate
//! (vendored under `tools/upscaler-vendor-sdks/`) plus this crate's
//! wrapper that implements [`engine_render::UpscalerProvider`] over the
//! real FFI.
//!
//! ## Default (no-feature) behaviour
//!
//! The default build (no cargo features) exposes only the type
//! signatures, not any real SDK calls. The trait-surface stubs that
//! Phase 5 PR 5 + Phase 6 PR 5 shipped in
//! [`engine_render::upscale`] remain authoritative for the default
//! build — every `supports()` returns false, the cascade falls
//! through to `OwnedBilinear`. This matches the pre-PR-5.5 behaviour
//! exactly so CI without SDKs / the default `cargo build` is
//! unchanged.
//!
//! ## Activated-feature behaviour
//!
//! When a vendor feature is enabled, the corresponding module
//! exposes a `real_supports(&Device)` helper that the application
//! (or the bench, or the runner) can wire into a custom
//! [`engine_render::UpscalerRegistry`] in place of the engine-render
//! stubs. The architectural decision in ADR-066 §6 has the registry
//! cascade pick the first provider whose `supports()` returns true;
//! the real `*_supports()` helpers honour each SDK's runtime feature
//! query (`slIsSupported` for DLSS, `ffxFsrContextCreate` probe for
//! FSR, `xessIsSupported` for XeSS, ORT session init for the owned
//! temporal upscaler).
//!
//! ## Why a separate crate
//!
//! Per ADR-066 §1 — engine-render must not link vendor SDKs directly.
//! The license-restricted blobs (DLSS Streamline, FSR runtime, XeSS
//! SDK) live behind feature gates here so:
//!
//! - The default workspace build pulls no vendor blobs.
//! - CI (without SDKs locally) builds identically to a vendor-less host.
//! - License compliance lives at this boundary (see ADR-066 §License
//!   management); engine-render's deny.toml stays vendor-agnostic.
//! - Future SDK upgrades / API breaks are contained to this crate; an
//!   ADR amendment + a PR replaces a per-vendor module without
//!   touching the renderer.
//!
//! ## Scaffold status (Phase 6 PR 5.5)
//!
//! The crate exists; the per-vendor modules are documented stubs.
//! The actual `*-sys` crate vendoring + FFI signatures + `unsafe`
//! call sites land in a follow-up PR that requires:
//!
//! - DLSS Streamline 2.x SDK downloaded + license accepted
//! - AMD FSR 4 SDK downloaded
//! - Intel XeSS 2 SDK downloaded
//! - ORT (ONNX Runtime) native binaries installed
//! - Git LFS configured for the bundled
//!   `crates/engine-render/assets/onnx/temporal_upscaler_v1.onnx`
//!   model file
//! - ADR-051 amended with the finalised ORT deviation entry
//!
//! Today this crate compiles into an empty library that re-exports
//! [`engine_render::UpscalerKind`] for caller convenience plus the
//! per-feature documentation that anchors the future PR.

pub use engine_render::UpscalerKind;

#[cfg(feature = "dlss")]
pub mod dlss;
#[cfg(feature = "fsr")]
pub mod fsr;
#[cfg(feature = "ort-runtime")]
pub mod ort_temporal;
#[cfg(feature = "xess")]
pub mod xess;

/// Build-info summary. Returned by the bench / runner to capture
/// which vendor features were compiled into a given build of the
/// engine-bench-frame-pacing binary or the editor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct VendorBuildInfo {
    /// `true` if the `dlss` cargo feature was active when the
    /// binary was built.
    pub dlss: bool,
    /// `true` if the `fsr` cargo feature was active.
    pub fsr: bool,
    /// `true` if the `xess` cargo feature was active.
    pub xess: bool,
    /// `true` if the `ort-runtime` cargo feature was active.
    pub ort_runtime: bool,
}

/// Query which vendor cargo features were enabled at build time.
///
/// Useful for the bench JSON report (ADR-066 §Consequences) so a
/// reader can tell which providers a given measurement was able to
/// exercise.
pub const fn build_info() -> VendorBuildInfo {
    VendorBuildInfo {
        dlss: cfg!(feature = "dlss"),
        fsr: cfg!(feature = "fsr"),
        xess: cfg!(feature = "xess"),
        ort_runtime: cfg!(feature = "ort-runtime"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_build_has_no_vendor_features() {
        // The default workspace build (no `--features ...` flags) is
        // the CI baseline + the "we have no SDK" build. Any vendor
        // feature being on at default build time would indicate a
        // misconfigured workspace.
        let info = build_info();
        assert!(!info.dlss);
        assert!(!info.fsr);
        assert!(!info.xess);
        assert!(!info.ort_runtime);
    }

    #[test]
    fn build_info_is_const_evaluable() {
        // `build_info` is `const fn` so the bench can emit it from a
        // `const` context if desired.
        const INFO: VendorBuildInfo = build_info();
        let runtime = build_info();
        assert_eq!(INFO, runtime);
    }

    #[test]
    fn upscaler_kind_re_export_round_trips() {
        // Ensure the re-export of `engine_render::UpscalerKind` works
        // end-to-end — readers should be able to consume this crate
        // and the kind enum without depending on engine-render directly.
        let k: UpscalerKind = UpscalerKind::OwnedBilinear;
        assert_eq!(k.name(), "owned.bilinear");
    }
}
