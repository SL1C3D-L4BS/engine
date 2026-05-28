//! GPU device + queue + tier-based limits.
//!
//! [`Device::new`] is the wgpu-init entry point. It walks the spec Part XX.7
//! hardware tiers via [`DeviceLimits`], requests a backend-agnostic adapter,
//! and produces an owned [`Device`] / [`Queue`] pair. Every subsequent
//! engine-gpu resource is created through these handles.

use crate::error::GpuError;
use crate::runtime::block_on;
use std::sync::Arc;

/// Hardware-tier configuration per spec Part XX.7 / ADR-049 §2.
///
/// The tier picks the [`wgpu::Limits`] table and required feature set. The
/// renderer reads the chosen tier back from [`Device::limits`] to decide
/// runtime quality (cascade count, IBL probe density, cluster grid size).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceLimits {
    /// RX 580 baseline — the Phase 5 milestone target. 16 K bindless slots,
    /// `downlevel_defaults` limits. The minimum spec-supported tier.
    Tier1Minimum,
    /// RTX 3060 / RX 6700 XT class. 65 K bindless slots, `default` wgpu
    /// limits, CSM 4096².
    Tier2Recommended,
    /// RTX 4080 / RX 7900 XTX class. 262 K bindless slots, push-constant
    /// support relied on.
    Tier3Enthusiast,
    /// Workstation / studio class. 1 M bindless slots; ray-tracing-capable
    /// features may be queried (not yet used by PR 2).
    Tier4AaaStudio,
}

impl DeviceLimits {
    /// Default bindless texture-heap capacity per tier (ADR-044 §1).
    pub fn bindless_texture_capacity(self) -> u32 {
        match self {
            DeviceLimits::Tier1Minimum => 16_384,
            DeviceLimits::Tier2Recommended => 65_536,
            DeviceLimits::Tier3Enthusiast => 262_144,
            DeviceLimits::Tier4AaaStudio => 1_048_576,
        }
    }

    fn wgpu_limits(self) -> wgpu::Limits {
        match self {
            DeviceLimits::Tier1Minimum => wgpu::Limits::downlevel_defaults(),
            DeviceLimits::Tier2Recommended => wgpu::Limits::default(),
            DeviceLimits::Tier3Enthusiast | DeviceLimits::Tier4AaaStudio => wgpu::Limits::default(),
        }
    }
}

/// Adapter-advertised feature flags relevant to the engine.
///
/// PR 2 surfaces the BC-codec flag (ADR-045 §1) and the bindless / push-
/// constant flags the renderer's hot path relies on. More flags land in PR
/// 3+ as their consumers do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct DeviceFeatures {
    /// `TEXTURE_COMPRESSION_BC` — required for ADR-045 BC{4,5,7}. Absence
    /// is a refuse-load condition per ADR-045 §1.
    pub bc_textures: bool,
    /// `BC6H` HDR — required for IBL specular probes (ADR-041; PR 4
    /// onward). Reported now so the renderer can branch.
    pub bc_hdr: bool,
    /// `PUSH_CONSTANTS` — required for ADR-044 §6's 8-byte per-draw
    /// (texture_id, sampler_id) push.
    pub push_constants: bool,
    /// `DESCRIPTOR_INDEXING_BINDING_*` (variable-length bindings) —
    /// required for bindless heap shader-side access per ADR-044
    /// "Risks and tradeoffs".
    pub descriptor_indexing: bool,
    /// `MULTIVIEW` (Vulkan `VK_KHR_multiview`, D3D12 view-instancing,
    /// Metal layered rendering). Required by the ADR-040 CSM shader's
    /// `@builtin(view_index)` — 4 cascades in 1 draw call. Polaris GFX8
    /// supports it via Mesa RADV; absence forces a fallback to 4 draws.
    pub multiview: bool,
    /// `TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES` — opt in to adapter-
    /// native format usages beyond the wgpu base spec. Required to use
    /// `R16Float` as a write-only storage texture (the SSAO target
    /// per ADR-065 §1). Polaris/RADV exposes R16Float storage write;
    /// older mobile / web adapters may not.
    pub adapter_specific_format_features: bool,
    /// `BGRA8UNORM_STORAGE` — write-only storage textures with format
    /// `Bgra8Unorm`. Required by the ADR-065 §6 tonemap pass which
    /// writes the final swapchain-compatible output. Polaris/RADV
    /// supports it natively (BGRA8 is the X11 / Wayland-preferred
    /// layout); absence forces an Rgba8Unorm intermediate + blit.
    pub bgra8unorm_storage: bool,
}

/// Owned GPU device handle.
///
/// Wraps `wgpu::Device` + `wgpu::Queue` + the chosen tier + the queried
/// feature set. Clone is cheap (`Arc` under the hood).
#[derive(Clone)]
pub struct Device {
    inner: Arc<DeviceInner>,
}

struct DeviceInner {
    // The `Instance` and `raw_instance` accessor below are consumed by
    // PR 3 (window-surface creation routed through the device); the
    // current PR 2 surface only needs the adapter / device pair for
    // headless work, but holding the instance keeps the device's
    // backend lifetime tidy.
    #[allow(dead_code, reason = "consumed by PR 3 surface creation")]
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    raw: wgpu::Device,
    queue: wgpu::Queue,
    limits: DeviceLimits,
    features: DeviceFeatures,
}

impl Device {
    /// Initialise wgpu against any backend the runner supports and request a
    /// device at the named tier. Falls back to a software adapter only if
    /// `allow_fallback` is `true` — headless CI runners pass `true`.
    ///
    /// **Caveat:** `allow_fallback` is the engine-gpu name for wgpu's
    /// `force_fallback_adapter` request. Passing `true` does not "allow
    /// fallback if no GPU is present" — it *requires* a software adapter
    /// (Lavapipe / SwiftShader / `wgpu` reference). Real-hardware
    /// consumers must pass `false`; only headless-CI-style harnesses
    /// that specifically want the software adapter should pass `true`.
    /// Renaming the parameter is a future API cleanup.
    pub fn new(limits: DeviceLimits, allow_fallback: bool) -> Result<Self, GpuError> {
        Self::new_inner(limits, allow_fallback, None)
    }

    /// Like [`Device::new`] but binds to a window surface eagerly so the
    /// adapter selection is constrained to surface-compatible adapters.
    ///
    /// The surface is consumed and a [`crate::Swapchain`] is returned
    /// alongside the device. PR 3+ uses this entry point when a window
    /// exists; offscreen / unit tests use [`Device::new`].
    pub fn new_with_surface(
        limits: DeviceLimits,
        surface: wgpu::Surface<'static>,
    ) -> Result<(Self, wgpu::Surface<'static>), GpuError> {
        // Same path as `new_inner`, but `compatible_surface` is set during
        // adapter selection so a discrete GPU compatible with the window is
        // chosen.
        let device = Self::new_inner(limits, false, Some(&surface))?;
        Ok((device, surface))
    }

    fn new_inner(
        limits: DeviceLimits,
        allow_fallback: bool,
        compatible_surface: Option<&wgpu::Surface<'_>>,
    ) -> Result<Self, GpuError> {
        let mut instance_desc = wgpu::InstanceDescriptor::new_without_display_handle();
        instance_desc.backends = wgpu::Backends::PRIMARY | wgpu::Backends::SECONDARY;
        let instance = wgpu::Instance::new(instance_desc);

        let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface,
            force_fallback_adapter: allow_fallback,
        }))
        .map_err(|_| GpuError::NoCompatibleAdapter {
            reason: "wgpu::Instance::request_adapter returned None",
        })?;

        let adapter_features = adapter.features();
        let features = DeviceFeatures {
            bc_textures: adapter_features.contains(wgpu::Features::TEXTURE_COMPRESSION_BC),
            bc_hdr: adapter_features.contains(wgpu::Features::TEXTURE_COMPRESSION_BC),
            // wgpu 29 renamed `PUSH_CONSTANTS` to `IMMEDIATES`. The
            // shader-side ABI in ADR-044 §6 uses the same primitive
            // (small fast per-draw data); we keep our public field name
            // `push_constants` for ADR-stability.
            push_constants: adapter_features.contains(wgpu::Features::IMMEDIATES),
            descriptor_indexing: adapter_features.contains(wgpu::Features::TEXTURE_BINDING_ARRAY)
                && adapter_features.contains(
                    wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING,
                ),
            multiview: adapter_features.contains(wgpu::Features::MULTIVIEW),
            adapter_specific_format_features: adapter_features
                .contains(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES),
            bgra8unorm_storage: adapter_features.contains(wgpu::Features::BGRA8UNORM_STORAGE),
        };

        let mut required_features = wgpu::Features::empty();
        if features.push_constants {
            required_features |= wgpu::Features::IMMEDIATES;
        }
        if features.bc_textures {
            required_features |= wgpu::Features::TEXTURE_COMPRESSION_BC;
        }
        if features.multiview {
            required_features |= wgpu::Features::MULTIVIEW;
        }
        if features.adapter_specific_format_features {
            required_features |= wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES;
        }
        if features.bgra8unorm_storage {
            required_features |= wgpu::Features::BGRA8UNORM_STORAGE;
        }

        // wgpu's `Limits::downlevel_defaults()` (Tier1Minimum) sets
        // `max_immediate_size = 0` even when the IMMEDIATES feature
        // is requested. The two are independent dials in wgpu's model:
        // the feature flag enables the API surface; the limit caps the
        // size. ADR-044 §6 uses 8 bytes; Vulkan's guaranteed-minimum
        // push-constant size is 128 bytes (every Polaris/RADV /
        // Skylake-GT / Apple Silicon device meets this). Raise the
        // limit to 128 when the feature is on so the engine's existing
        // pipelines (8-byte push for the texture_id + sampler_id pair)
        // construct cleanly. (wgpu 29 renamed `PUSH_CONSTANTS` →
        // `IMMEDIATES` and `max_push_constant_size` → `max_immediate_size`;
        // the engine keeps the legacy `push_constants` field name for
        // ADR-stability per ADR-044.) ADR-074 §3.
        let mut required_limits = limits.wgpu_limits();
        if features.push_constants {
            required_limits.max_immediate_size = 128;
        }

        let device_desc = wgpu::DeviceDescriptor {
            label: Some("engine-gpu device"),
            required_features,
            required_limits,
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
        };

        let (raw, queue) = block_on(adapter.request_device(&device_desc)).map_err(|e| {
            GpuError::DeviceCreationFailed {
                reason: e.to_string(),
            }
        })?;

        Ok(Self {
            inner: Arc::new(DeviceInner {
                instance,
                adapter,
                raw,
                queue,
                limits,
                features,
            }),
        })
    }

    /// Chosen hardware tier.
    pub fn limits(&self) -> DeviceLimits {
        self.inner.limits
    }

    /// Adapter-advertised feature flags.
    pub fn features(&self) -> DeviceFeatures {
        self.inner.features
    }

    /// Borrow the underlying `wgpu::Device`. Crate-internal only.
    pub(crate) fn raw(&self) -> &wgpu::Device {
        &self.inner.raw
    }

    /// Borrow the underlying `wgpu::Queue`. Crate-internal only.
    pub(crate) fn raw_queue(&self) -> &wgpu::Queue {
        &self.inner.queue
    }

    /// Borrow the underlying `wgpu::Instance`. Crate-internal only;
    /// consumed by PR 3's window-surface creation path.
    #[allow(dead_code, reason = "consumed by PR 3 surface creation")]
    pub(crate) fn raw_instance(&self) -> &wgpu::Instance {
        &self.inner.instance
    }

    /// Borrow the underlying `wgpu::Adapter`. Crate-internal only — used by
    /// the swapchain to query surface capabilities.
    pub(crate) fn raw_adapter(&self) -> &wgpu::Adapter {
        &self.inner.adapter
    }

    /// Owned [`Queue`] handle. Clone is cheap — wraps `Arc<DeviceInner>`.
    pub fn queue(&self) -> Queue {
        Queue {
            device: self.clone(),
        }
    }
}

impl core::fmt::Debug for Device {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Device")
            .field("limits", &self.inner.limits)
            .field("features", &self.inner.features)
            .finish_non_exhaustive()
    }
}

/// Owned GPU submission queue.
///
/// Cheap clone; identity-equal across copies (Arc-backed).
#[derive(Clone, Debug)]
pub struct Queue {
    device: Device,
}

impl Queue {
    /// Submit a command buffer. Returns a [`crate::SubmitToken`] the caller
    /// can use to wait for completion if needed.
    pub fn submit(&self, encoder: crate::CommandEncoder) -> crate::SubmitToken {
        let buf = encoder.finish_raw();
        let idx = self.device.raw_queue().submit(std::iter::once(buf));
        crate::SubmitToken::new(idx)
    }

    /// Write `data` to `buffer` at byte offset `offset`. Convenience for
    /// staging-free uploads via wgpu's internal staging belt.
    pub fn write_buffer(&self, buffer: &crate::Buffer, offset: u64, data: &[u8]) {
        self.device
            .raw_queue()
            .write_buffer(buffer.raw(), offset, data);
    }

    /// Upload `data` to a 2D texture's mip 0 base layer.
    ///
    /// `bytes_per_row` must align with wgpu's COPY_BYTES_PER_ROW_ALIGNMENT
    /// (256) on native backends; BC-compressed paks already align (16 KB
    /// block stride for BC7 at 4 K width). The texture must have been
    /// created with [`crate::TextureUsage::COPY_DST`].
    pub fn write_texture_2d(
        &self,
        texture: &crate::Texture,
        data: &[u8],
        bytes_per_row: u32,
        rows_per_image: u32,
    ) {
        let extent = texture.extent();
        self.device.raw_queue().write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: texture.raw(),
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(rows_per_image),
            },
            wgpu::Extent3d {
                width: extent.width,
                height: extent.height,
                depth_or_array_layers: extent.depth_or_array_layers,
            },
        );
    }

    /// Owning [`Device`] handle.
    pub fn device(&self) -> &Device {
        &self.device
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bindless_capacity_per_tier_matches_adr_044() {
        assert_eq!(
            DeviceLimits::Tier1Minimum.bindless_texture_capacity(),
            16_384
        );
        assert_eq!(
            DeviceLimits::Tier2Recommended.bindless_texture_capacity(),
            65_536
        );
        assert_eq!(
            DeviceLimits::Tier3Enthusiast.bindless_texture_capacity(),
            262_144
        );
        assert_eq!(
            DeviceLimits::Tier4AaaStudio.bindless_texture_capacity(),
            1_048_576
        );
    }

    #[test]
    fn capacity_strictly_increases_with_tier() {
        let tiers = [
            DeviceLimits::Tier1Minimum,
            DeviceLimits::Tier2Recommended,
            DeviceLimits::Tier3Enthusiast,
            DeviceLimits::Tier4AaaStudio,
        ];
        for w in tiers.windows(2) {
            assert!(
                w[0].bindless_texture_capacity() < w[1].bindless_texture_capacity(),
                "tier capacity must be monotonically increasing"
            );
        }
    }

    #[test]
    fn default_features_are_empty() {
        let f = DeviceFeatures::default();
        assert!(!f.bc_textures);
        assert!(!f.bc_hdr);
        assert!(!f.push_constants);
        assert!(!f.descriptor_indexing);
    }
}
