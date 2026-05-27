//! Intel XeSS 2 binding stub. Active only when the `xess` cargo
//! feature is enabled.
//!
//! ## Scaffold status (Phase 6 PR 5.5)
//!
//! This module is the future home for the XeSS provider impl. Today
//! it is a documented stub — the real binding lands in a follow-up
//! PR that has Intel XeSS SDK access. The follow-up PR adds:
//!
//! - `xess-sys` crate vendored at
//!   `tools/upscaler-vendor-sdks/xess/`. Minimal `bindgen`-generated
//!   FFI from the XeSS SDK's public C headers.
//! - `struct VendorXessReal` that implements
//!   [`engine_render::UpscalerProvider`] by calling
//!   `xessCreateContext()` once at construction and `xessExecute()`
//!   per frame.
//! - `supports()` returns `true` on every GPU XeSS recognises (its
//!   own internal feature detection via `xessIsSupported`).
//! - A `LICENSE-VENDOR.txt` mirroring the Intel XeSS SDK license,
//!   plus a `deny.toml` `[[licenses.exceptions]]` entry for
//!   `LicenseRef-Intel-XeSS-EULA`.
//!
//! When the follow-up lands, this stub becomes the real binding and
//! `engine_render::upscale::VendorXess::supports()` may delegate to
//! a `cfg(feature = "xess")` re-export of `VendorXessReal::supports()`
//! defined here.

use engine_render::UpscalerKind;

/// Identifies this provider's vendor at the trait boundary.
pub const KIND: UpscalerKind = UpscalerKind::Xess;

/// Stub probe — always returns false in the PR-5.5 scaffold; the
/// real implementation lands when XeSS SDK access is available.
pub fn supports_stub() -> bool {
    false
}
