//! AMD FidelityFX FSR 4 FFI bindings (ADR-079).
//!
//! **Scaffold module** — see the runbook for the SDK fetch
//! procedure. The bindgen-generated bindings replace this scaffold
//! once `ffx-sdk/` is vendored.

#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(missing_docs)]

/// Opaque FFX FSR 4 context handle.
#[repr(transparent)]
#[derive(Clone, Copy, Debug)]
pub struct FfxFsr4Context(pub u64);

/// FFX result enum (success / failure).
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FfxResult {
    Ok = 0,
    /// The active GPU does not support FSR 4's tensor path (RDNA 4
    /// or newer); fall back to FSR-EASU per ADR-076.
    UnsupportedDevice = 1,
    /// FFX runtime failed to initialize.
    InitFailed = 2,
}

/// Probe whether the device supports the FSR 4 tensor path. Until
/// the SDK is vendored, returns `FfxResult::UnsupportedDevice` so
/// the cascade falls through to EASU.
///
/// # Safety
///
/// FFI boundary — the caller must ensure the FFX runtime is
/// loadable. Use `engine-upscale-vendor::loader::VendorLoader` per
/// ADR-079 §4.
pub unsafe fn ffxFsr4ContextProbe() -> FfxResult {
    FfxResult::UnsupportedDevice
}

/// FFX build-info string.
pub const fn fsr4_build_info() -> &'static str {
    "fsr4-sys: scaffold (no SDK vendored)"
}
