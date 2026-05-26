//! Window-surface swapchain wrapper.
//!
//! Wraps `wgpu::Surface` + `wgpu::SurfaceConfiguration`. The owned
//! [`Swapchain`] exposes resize / acquire / present without leaking wgpu
//! identifiers. PR 2's tests exercise resize on an offscreen surface
//! created from a synthetic window handle when a runner advertises one;
//! otherwise the test is gracefully skipped.

use crate::device::Device;
use crate::error::GpuError;
use crate::texture::{TextureFormat, TextureView};

/// Vsync / present-mode choice. Mirrors `wgpu::PresentMode` semantically
/// — the renderer uses [`PresentMode::Mailbox`] when it's available
/// (variable-refresh hardware) and falls back to [`PresentMode::Fifo`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresentMode {
    /// Strict v-sync (queue depth ≤ 2). The portable default.
    Fifo,
    /// Tear-free triple-buffered. Lower latency than Fifo; not always
    /// supported.
    Mailbox,
    /// Immediate present, no v-sync. May tear. Used by bench scenarios.
    Immediate,
}

impl PresentMode {
    fn to_wgpu(self) -> wgpu::PresentMode {
        match self {
            PresentMode::Fifo => wgpu::PresentMode::Fifo,
            PresentMode::Mailbox => wgpu::PresentMode::Mailbox,
            PresentMode::Immediate => wgpu::PresentMode::Immediate,
        }
    }
}

/// Swapchain configuration. Carried explicitly so resize is a pure
/// recompute on the owned state and the wgpu reconfigure is the last step.
#[derive(Clone, Copy, Debug)]
pub struct SwapchainConfig {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Surface texture format.
    pub format: TextureFormat,
    /// Present mode.
    pub present_mode: PresentMode,
}

/// Owned window-surface swapchain.
///
/// PR 2's render-graph hookup is minimal: callers acquire the current
/// surface texture via [`Swapchain::acquire`], record passes against the
/// returned [`SwapchainTexture::view`], and call [`SwapchainTexture::present`]
/// to flip.
pub struct Swapchain {
    surface: wgpu::Surface<'static>,
    device: Device,
    config: SwapchainConfig,
}

impl Swapchain {
    /// Build a swapchain over an already-created `wgpu::Surface`.
    ///
    /// Use [`Device::new_with_surface`](crate::Device::new_with_surface) to
    /// obtain the surface + device pair in one call.
    pub fn new(
        device: Device,
        surface: wgpu::Surface<'static>,
        config: SwapchainConfig,
    ) -> Result<Self, GpuError> {
        let s = Self {
            surface,
            device,
            config,
        };
        s.reconfigure()?;
        Ok(s)
    }

    /// Current configuration (post-last-resize).
    pub fn config(&self) -> SwapchainConfig {
        self.config
    }

    /// Resize the swapchain. `wgpu::SurfaceConfiguration` rebuilt; the new
    /// dimensions take effect on the next [`Swapchain::acquire`].
    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), GpuError> {
        self.config.width = width.max(1);
        self.config.height = height.max(1);
        self.reconfigure()
    }

    fn reconfigure(&self) -> Result<(), GpuError> {
        let caps = self.surface.get_capabilities(self.device.raw_adapter());
        let preferred = self.config.present_mode.to_wgpu();
        let present_mode = if caps.present_modes.contains(&preferred) {
            preferred
        } else {
            wgpu::PresentMode::Fifo
        };
        let alpha_mode = caps
            .alpha_modes
            .first()
            .copied()
            .unwrap_or(wgpu::CompositeAlphaMode::Auto);
        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: self.config.format.to_wgpu(),
            width: self.config.width,
            height: self.config.height,
            present_mode,
            desired_maximum_frame_latency: 2,
            alpha_mode,
            view_formats: vec![],
        };
        self.surface.configure(self.device.raw(), &surface_config);
        Ok(())
    }

    /// Acquire the current backbuffer texture.
    pub fn acquire(&self) -> Result<SwapchainTexture, GpuError> {
        // wgpu 29 returns the `CurrentSurfaceTexture` enum rather than a
        // `Result`. `Suboptimal` is treated as `Success` here — the caller
        // is expected to reconfigure at the next resize.
        let (frame, _suboptimal) = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) => (f, false),
            wgpu::CurrentSurfaceTexture::Suboptimal(f) => (f, true),
            wgpu::CurrentSurfaceTexture::Timeout => return Err(GpuError::SwapchainTimeout),
            wgpu::CurrentSurfaceTexture::Occluded => return Err(GpuError::SwapchainTimeout),
            wgpu::CurrentSurfaceTexture::Outdated => return Err(GpuError::SwapchainOutdated),
            wgpu::CurrentSurfaceTexture::Lost => return Err(GpuError::SwapchainLost),
            wgpu::CurrentSurfaceTexture::Validation => return Err(GpuError::SwapchainLost),
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        Ok(SwapchainTexture {
            inner: Some(frame),
            view,
        })
    }
}

impl core::fmt::Debug for Swapchain {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Swapchain")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

/// Backbuffer texture handed back from [`Swapchain::acquire`].
///
/// On drop without `present()` the frame is discarded (wgpu's default
/// behaviour). PR 2 callers should always call [`SwapchainTexture::present`]
/// after recording.
pub struct SwapchainTexture {
    inner: Option<wgpu::SurfaceTexture>,
    view: wgpu::TextureView,
}

impl SwapchainTexture {
    /// Borrow the view to record passes against.
    pub fn view(&self) -> TextureView<'_> {
        TextureView::from_raw(&self.view)
    }

    /// Present the backbuffer.
    pub fn present(mut self) {
        if let Some(frame) = self.inner.take() {
            frame.present();
        }
    }
}

impl core::fmt::Debug for SwapchainTexture {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SwapchainTexture").finish_non_exhaustive()
    }
}
