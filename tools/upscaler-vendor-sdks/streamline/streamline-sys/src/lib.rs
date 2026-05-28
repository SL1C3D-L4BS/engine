//! NVIDIA Streamline 2.x FFI bindings (ADR-079).
//!
//! **Scaffold module.** The full bindgen-generated bindings land
//! with the SDK fetch + verification step documented in
//! `docs/runbooks/vendor-upscaler-sdks.md`. Until the SDK is
//! vendored, this module exposes the *types* the
//! `engine-upscale-vendor::dlss` consumer needs so the integration
//! site compiles + can be unit-tested with feature gating off.
//!
//! When the SDK lands, this file is *regenerated* by `build.rs`
//! invoking `bindgen` against the vendored headers; the committed
//! shape stays the same (caller sites do not change), and a digest
//! verification step in `build.rs` rejects an SDK that has been
//! modified vs. the pinned `BLAKE3.txt` manifest.

#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(missing_docs)]

/// Opaque Streamline frame token. The real SDK exposes this as
/// `sl::FrameToken` (a C++ POD); the FFI surface treats it as
/// opaque.
#[repr(transparent)]
#[derive(Clone, Copy, Debug)]
pub struct sl_FrameToken(pub u64);

/// DLSS feature query result. Mirrors `sl::Result` (the SDK's
/// success/failure enum).
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum sl_Result {
    /// Operation succeeded.
    Ok = 0,
    /// Streamline loader was not initialized.
    NoLoader = 1,
    /// Adapter does not support the requested feature.
    Unsupported = 2,
    /// Loader returned a failure (driver lost, OOM, etc.).
    Failed = 3,
}

/// `sl::Feature` discriminants for the features this engine binds.
/// Mirrors the SDK enum.
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum sl_Feature {
    /// DLSS Super Resolution (the upscaler).
    DLSS = 0,
    /// DLSS Frame Generation (Phase 7+, not yet bound).
    DLSS_G = 1,
    /// Reflex low-latency.
    Reflex = 2,
}

/// Probe whether the active adapter supports a given feature.
/// Until the SDK lands, the scaffolded stub returns
/// `sl_Result::NoLoader`.
///
/// # Safety
///
/// FFI boundary — the caller must ensure the Streamline runtime is
/// loadable and the device handle is valid. The
/// `engine-upscale-vendor::loader::VendorLoader` wrapper enforces
/// this discipline.
pub unsafe fn slIsSupported(_feature: sl_Feature) -> sl_Result {
    sl_Result::NoLoader
}

/// SDK build-info string. The runtime path uses this for telemetry.
pub const fn streamline_build_info() -> &'static str {
    "streamline-sys: scaffold (no SDK vendored)"
}
