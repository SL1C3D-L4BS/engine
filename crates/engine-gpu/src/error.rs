//! Owned GPU error enum.
//!
//! Every fallible call in `engine-gpu` returns `Result<_, GpuError>`. wgpu's
//! own error types are flattened into this owned enum so callers never name
//! `wgpu::*` types (ADR-049 §3).

use core::fmt;

/// All ways `engine-gpu` can fail at construction or runtime.
///
/// Variants are intentionally coarse: PR 2 surfaces only the errors the
/// renderer needs to distinguish (no compatible adapter, request-device
/// rejected, BC support absent, swapchain lost). Finer-grained surfaces
/// land alongside the passes that need them (PR 3 onward).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GpuError {
    /// No GPU adapter matched the requested [`crate::DeviceLimits`] tier.
    /// On a CI runner without a Vulkan / Metal / DX12 driver, every
    /// software backend is also unable to satisfy the request — the engine
    /// reports the runner as below the supported tier.
    NoCompatibleAdapter {
        /// Human-readable description of what failed (carried as a static
        /// string to keep [`GpuError`] cheap to clone).
        reason: &'static str,
    },
    /// `Adapter::request_device` rejected the descriptor. Usually because
    /// the requested feature set is unavailable on the picked adapter.
    DeviceCreationFailed {
        /// String form of the underlying wgpu error.
        reason: String,
    },
    /// The runtime adapter advertises no support for
    /// `TEXTURE_COMPRESSION_BC`. Per ADR-045 §1 this is a refuse-load
    /// condition: paks contain BC bytes only, so the engine reports the
    /// hardware as below tier rather than silently de-quality.
    BcSupportAbsent,
    /// The swapchain surface returned `SurfaceError::Lost` and the renderer
    /// must reconfigure / resize before the next frame.
    SwapchainLost,
    /// The swapchain surface returned `SurfaceError::OutOfMemory`. A fatal
    /// condition; the caller should terminate the frame loop.
    SwapchainOutOfMemory,
    /// The swapchain surface returned `SurfaceError::Outdated`. The caller
    /// should reconfigure and retry.
    SwapchainOutdated,
    /// The swapchain surface returned `SurfaceError::Timeout`. Treated as
    /// a transient skip-the-frame condition.
    SwapchainTimeout,
    /// Returned by [`crate::Buffer::read_back`] when the GPU mapping yields
    /// an error.
    BufferMapFailed {
        /// String form of the underlying wgpu error.
        reason: String,
    },
}

impl fmt::Display for GpuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GpuError::NoCompatibleAdapter { reason } => {
                write!(f, "no compatible GPU adapter: {reason}")
            }
            GpuError::DeviceCreationFailed { reason } => {
                write!(f, "device creation failed: {reason}")
            }
            GpuError::BcSupportAbsent => {
                f.write_str("adapter does not advertise TEXTURE_COMPRESSION_BC (ADR-045 §1)")
            }
            GpuError::SwapchainLost => f.write_str("swapchain surface lost — reconfigure"),
            GpuError::SwapchainOutOfMemory => f.write_str("swapchain out of memory"),
            GpuError::SwapchainOutdated => f.write_str("swapchain outdated — reconfigure"),
            GpuError::SwapchainTimeout => f.write_str("swapchain timeout (transient)"),
            GpuError::BufferMapFailed { reason } => write!(f, "buffer map failed: {reason}"),
        }
    }
}

impl std::error::Error for GpuError {}
