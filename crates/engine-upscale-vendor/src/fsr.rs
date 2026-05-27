//! AMD FSR 4 binding stub. Active only when the `fsr` cargo feature
//! is enabled.
//!
//! ## Scaffold status (Phase 6 PR 5.5)
//!
//! This module is the future home for the FSR provider impl. Today
//! it is a documented stub — the real binding lands in a follow-up
//! PR that has AMD GPUOpen SDK access. The follow-up PR adds:
//!
//! - `fsr-sys` crate vendored at
//!   `tools/upscaler-vendor-sdks/fsr/`. Minimal `bindgen`-generated
//!   FFI from the AMD FidelityFX SDK's public C headers.
//! - `struct VendorFsrReal` that implements
//!   [`engine_render::UpscalerProvider`] by calling
//!   `ffxFsrContextCreate()` once at construction and
//!   `ffxFsrDispatch()` per frame.
//! - `supports()` returns `true` on RDNA 4 + tensor path, or any
//!   DX12 / Vulkan device for the FSR 3.x spatial fallback.
//! - A `LICENSE-VENDOR.txt` mirroring the AMD FidelityFX SDK
//!   license, plus a `deny.toml` `[[licenses.exceptions]]` entry
//!   for `LicenseRef-AMD-FSR-EULA`.
//!
//! When the follow-up lands, this stub becomes the real binding and
//! `engine_render::upscale::VendorFsr::supports()` may delegate to a
//! `cfg(feature = "fsr")` re-export of `VendorFsrReal::supports()`
//! defined here.

use engine_render::UpscalerKind;

/// Identifies this provider's vendor at the trait boundary.
pub const KIND: UpscalerKind = UpscalerKind::Fsr;

/// Stub probe — always returns false in the PR-5.5 scaffold; the
/// real implementation lands when FSR 4 SDK access is available.
pub fn supports_stub() -> bool {
    false
}
