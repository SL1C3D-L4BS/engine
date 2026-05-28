//! Intel XeSS 2 FFI bindings (ADR-079).
//!
//! **Scaffold module** — see the runbook for the SDK fetch
//! procedure. The bindgen-generated bindings replace this scaffold
//! once `xess-sdk/` is vendored.

#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(missing_docs)]

/// Opaque XeSS 2 context handle.
#[repr(transparent)]
#[derive(Clone, Copy, Debug)]
pub struct xess_context_handle_t(pub u64);

/// XeSS result enum.
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum xess_result_t {
    Ok = 0,
    /// Adapter has no XMX path (DP4a fallback may still be present;
    /// caller can re-probe via `xessIsSupportedDP4a`).
    UnsupportedXmx = 1,
    /// No DP4a path either; fully unsupported on this device.
    UnsupportedDevice = 2,
    InitFailed = 3,
}

/// Probe whether the device supports the XMX-accelerated XeSS path
/// (Arc B+ / Intel Battlemage and newer).
///
/// # Safety
///
/// FFI boundary — load XeSS via the loader-thread sandbox.
pub unsafe fn xessIsSupported() -> xess_result_t {
    xess_result_t::UnsupportedDevice
}

/// Probe whether the device supports the DP4a cross-vendor XeSS
/// path. Returns `Ok` on any GPU with DP4a (most NVIDIA Turing+ and
/// AMD RDNA+); `UnsupportedDevice` otherwise.
///
/// # Safety
///
/// FFI boundary — same caveats as `xessIsSupported`.
pub unsafe fn xessIsSupportedDP4a() -> xess_result_t {
    xess_result_t::UnsupportedDevice
}

/// XeSS SDK build-info string.
pub const fn xess2_build_info() -> &'static str {
    "xess2-sys: scaffold (no SDK vendored)"
}
