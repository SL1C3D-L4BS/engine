//! NVIDIA DLSS (Streamline 2.x) binding stub. Active only when the
//! `dlss` cargo feature is enabled.
//!
//! ## Scaffold status (Phase 6 PR 5.5)
//!
//! This module is the future home for the DLSS provider impl. Today
//! it is a documented stub — the real binding lands in a follow-up
//! PR that has SDK access. The follow-up PR adds:
//!
//! - `streamline-sys` crate vendored at
//!   `tools/upscaler-vendor-sdks/streamline/`. Minimal `bindgen`-
//!   generated FFI from Streamline's `sl.h`. Committed `bindings.rs`
//!   so the build doesn't depend on `bindgen` at compile time.
//! - `struct VendorDlssReal` that implements
//!   [`engine_render::UpscalerProvider`] by calling `slInit()` once
//!   at construction (under `catch_unwind` per ADR-066 §3) and
//!   `slEvaluateFeature(sl::Feature::DLSS, ...)` per frame.
//! - `supports()` returns `true` on RTX 20+ / 40+ / 50+ with a
//!   Streamline-loadable driver. NVIDIA's own probe (`slIsSupported`)
//!   is the source of truth.
//! - A `LICENSE-VENDOR.txt` mirroring the NVIDIA Streamline SDK
//!   license, plus a `deny.toml` `[[licenses.exceptions]]` entry
//!   for `LicenseRef-NVIDIA-Streamline`.
//! - The SDK shared-library digest (BLAKE3) checked at engine init
//!   against `crates/engine-upscale-vendor/sdk_digests.toml` so a
//!   tampered library is detected before any FFI call.
//!
//! When the follow-up lands, this stub becomes the real binding and
//! `engine_render::upscale::VendorDlss::supports()` may delegate to
//! a `cfg(feature = "dlss")` re-export of `VendorDlssReal::supports()`
//! defined here.

use engine_render::UpscalerKind;

/// Identifies this provider's vendor at the trait boundary.
pub const KIND: UpscalerKind = UpscalerKind::Dlss;

/// Stub probe — always returns false in the PR-5.5 scaffold; the
/// real implementation lands when Streamline 2.x SDK access is
/// available.
pub fn supports_stub() -> bool {
    false
}
