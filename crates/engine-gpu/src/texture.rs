//! GPU texture wrapper.
//!
//! Owned [`Texture`] + [`TextureView`] + [`TextureDesc`] + [`TextureFormat`]
//! covering the formats PR 2 needs (BC{4,5,6,7} per ADR-045, plus the
//! uncompressed formats the rasterizer / TAA paths require).

use crate::device::Device;

/// Width × height × depth of a 2D / 3D resource.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Extent3d {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Depth in pixels (1 for 2D textures) / layer count for 2D arrays.
    pub depth_or_array_layers: u32,
}

impl Extent3d {
    /// 2D extent helper.
    pub const fn new_2d(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            depth_or_array_layers: 1,
        }
    }

    fn to_wgpu(self) -> wgpu::Extent3d {
        wgpu::Extent3d {
            width: self.width,
            height: self.height,
            depth_or_array_layers: self.depth_or_array_layers,
        }
    }
}

/// Texture dimension. PR 2 ships 2D + 2D-array + Cube; 3D lands with the
/// volumetric / IBL probe work in PR 4.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextureDimension {
    /// Standard 2D texture.
    D2,
    /// 2D layered (texture array).
    D2Array,
    /// Cube map (six 2D faces).
    Cube,
    /// 3D volume.
    D3,
}

impl TextureDimension {
    fn to_wgpu(self) -> wgpu::TextureDimension {
        match self {
            TextureDimension::D2 | TextureDimension::D2Array | TextureDimension::Cube => {
                wgpu::TextureDimension::D2
            }
            TextureDimension::D3 => wgpu::TextureDimension::D3,
        }
    }

    fn view_dimension(self) -> wgpu::TextureViewDimension {
        match self {
            TextureDimension::D2 => wgpu::TextureViewDimension::D2,
            TextureDimension::D2Array => wgpu::TextureViewDimension::D2Array,
            TextureDimension::Cube => wgpu::TextureViewDimension::Cube,
            TextureDimension::D3 => wgpu::TextureViewDimension::D3,
        }
    }
}

/// Texture pixel format.
///
/// Owned enum mirroring the subset of `wgpu::TextureFormat` PR 2 needs. The
/// BC formats (ADR-045 §2) are the core deliverable; the uncompressed
/// formats cover G-buffer targets (PR 3), the depth atlas (PR 3), and the
/// swapchain (any PR).
///
/// Variants not listed here are not engine surfaces — new variants require
/// an explicit reason (an ADR amendment, or a passing PR that
/// demonstrates need).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TextureFormat {
    // -- swapchain / G-buffer ------------------------------------------
    /// 8-bit RGBA, sRGB-encoded. Default swapchain format.
    Rgba8UnormSrgb,
    /// 8-bit RGBA, linear.
    Rgba8Unorm,
    /// 8-bit BGRA, sRGB-encoded. Common Windows / macOS swapchain default.
    Bgra8UnormSrgb,
    /// 16-bit float RGBA. HDR intermediate.
    Rgba16Float,
    /// 32-bit float depth.
    Depth32Float,
    /// 32-bit float depth + 8-bit stencil.
    Depth32FloatStencil8,
    /// 24-bit depth + 8-bit stencil.
    Depth24PlusStencil8,
    // -- BC codecs (ADR-045 §2) ----------------------------------------
    /// BC4 single-channel — used for roughness / metallic / AO masks.
    /// 8 bytes per 4×4 block.
    Bc4RUnorm,
    /// BC5 two-channel — used for tangent-space normals (Z reconstructed in
    /// shader). 16 bytes per 4×4 block.
    Bc5RgUnorm,
    /// BC6H unsigned-float — used for HDR cubemaps / IBL specular probes.
    /// 16 bytes per 4×4 block.
    Bc6hRgbUfloat,
    /// BC7 RGBA, sRGB-encoded. Used for albedo / diffuse / UI.
    /// 16 bytes per 4×4 block.
    Bc7RgbaUnormSrgb,
    /// BC7 RGBA, linear. Used for packed roughness+metallic+AO maps.
    Bc7RgbaUnorm,
}

impl TextureFormat {
    /// Convert to `wgpu::TextureFormat`. Crate-internal.
    pub(crate) fn to_wgpu(self) -> wgpu::TextureFormat {
        match self {
            TextureFormat::Rgba8UnormSrgb => wgpu::TextureFormat::Rgba8UnormSrgb,
            TextureFormat::Rgba8Unorm => wgpu::TextureFormat::Rgba8Unorm,
            TextureFormat::Bgra8UnormSrgb => wgpu::TextureFormat::Bgra8UnormSrgb,
            TextureFormat::Rgba16Float => wgpu::TextureFormat::Rgba16Float,
            TextureFormat::Depth32Float => wgpu::TextureFormat::Depth32Float,
            TextureFormat::Depth32FloatStencil8 => wgpu::TextureFormat::Depth32FloatStencil8,
            TextureFormat::Depth24PlusStencil8 => wgpu::TextureFormat::Depth24PlusStencil8,
            TextureFormat::Bc4RUnorm => wgpu::TextureFormat::Bc4RUnorm,
            TextureFormat::Bc5RgUnorm => wgpu::TextureFormat::Bc5RgUnorm,
            TextureFormat::Bc6hRgbUfloat => wgpu::TextureFormat::Bc6hRgbUfloat,
            TextureFormat::Bc7RgbaUnormSrgb => wgpu::TextureFormat::Bc7RgbaUnormSrgb,
            TextureFormat::Bc7RgbaUnorm => wgpu::TextureFormat::Bc7RgbaUnorm,
        }
    }

    /// `true` when this format is in the BC{4,5,6,7} family (ADR-045 §2).
    /// Used by the renderer to gate refuse-load on adapters that don't
    /// advertise [`crate::DeviceFeatures::bc_textures`].
    pub fn is_bc(self) -> bool {
        matches!(
            self,
            TextureFormat::Bc4RUnorm
                | TextureFormat::Bc5RgUnorm
                | TextureFormat::Bc6hRgbUfloat
                | TextureFormat::Bc7RgbaUnormSrgb
                | TextureFormat::Bc7RgbaUnorm
        )
    }

    /// `true` when this format encodes depth (used for view aspect choice).
    pub fn is_depth(self) -> bool {
        matches!(
            self,
            TextureFormat::Depth32Float
                | TextureFormat::Depth32FloatStencil8
                | TextureFormat::Depth24PlusStencil8
        )
    }
}

/// Allowed usages for a [`Texture`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct TextureUsage(u32);

impl TextureUsage {
    /// Empty set.
    pub const EMPTY: TextureUsage = TextureUsage(0);
    /// Source of a copy.
    pub const COPY_SRC: TextureUsage = TextureUsage(1 << 0);
    /// Destination of a copy (texture upload target).
    pub const COPY_DST: TextureUsage = TextureUsage(1 << 1);
    /// Bindable as a shader-resource view (sampled in fragments / compute).
    pub const TEXTURE_BINDING: TextureUsage = TextureUsage(1 << 2);
    /// Bindable as a storage texture (compute writes).
    pub const STORAGE_BINDING: TextureUsage = TextureUsage(1 << 3);
    /// Render-target attachment.
    pub const RENDER_ATTACHMENT: TextureUsage = TextureUsage(1 << 4);

    /// Set union.
    pub const fn union(self, other: TextureUsage) -> TextureUsage {
        TextureUsage(self.0 | other.0)
    }

    /// Membership test.
    pub const fn contains(self, other: TextureUsage) -> bool {
        (self.0 & other.0) == other.0
    }

    fn to_wgpu(self) -> wgpu::TextureUsages {
        let mut u = wgpu::TextureUsages::empty();
        if self.contains(Self::COPY_SRC) {
            u |= wgpu::TextureUsages::COPY_SRC;
        }
        if self.contains(Self::COPY_DST) {
            u |= wgpu::TextureUsages::COPY_DST;
        }
        if self.contains(Self::TEXTURE_BINDING) {
            u |= wgpu::TextureUsages::TEXTURE_BINDING;
        }
        if self.contains(Self::STORAGE_BINDING) {
            u |= wgpu::TextureUsages::STORAGE_BINDING;
        }
        if self.contains(Self::RENDER_ATTACHMENT) {
            u |= wgpu::TextureUsages::RENDER_ATTACHMENT;
        }
        u
    }
}

impl core::ops::BitOr for TextureUsage {
    type Output = TextureUsage;
    fn bitor(self, rhs: Self) -> Self::Output {
        self.union(rhs)
    }
}

impl core::ops::BitOrAssign for TextureUsage {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

/// Texture descriptor.
#[derive(Clone, Debug)]
pub struct TextureDesc<'a> {
    /// Debug label.
    pub label: &'a str,
    /// Dimensions.
    pub extent: Extent3d,
    /// Mip-level count. `1` disables mipping.
    pub mip_level_count: u32,
    /// Sample count (1 for non-MSAA).
    pub sample_count: u32,
    /// Texture dimension.
    pub dimension: TextureDimension,
    /// Pixel format.
    pub format: TextureFormat,
    /// Allowed usages.
    pub usage: TextureUsage,
}

impl TextureDesc<'_> {
    /// Standard 2D albedo descriptor (BC7 sRGB, full mip chain, sampled).
    pub fn albedo_2d(label: &str, width: u32, height: u32, mips: u32) -> TextureDesc<'_> {
        TextureDesc {
            label,
            extent: Extent3d::new_2d(width, height),
            mip_level_count: mips,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Bc7RgbaUnormSrgb,
            usage: TextureUsage::TEXTURE_BINDING | TextureUsage::COPY_DST,
        }
    }
}

/// Owned GPU texture.
#[derive(Debug)]
pub struct Texture {
    raw: wgpu::Texture,
    default_view: wgpu::TextureView,
    format: TextureFormat,
    extent: Extent3d,
    dimension: TextureDimension,
    mip_level_count: u32,
    device: Device,
}

impl Texture {
    /// Create a texture through the device.
    pub fn new(device: &Device, desc: &TextureDesc<'_>) -> Self {
        let raw = device.raw().create_texture(&wgpu::TextureDescriptor {
            label: Some(desc.label),
            size: desc.extent.to_wgpu(),
            mip_level_count: desc.mip_level_count,
            sample_count: desc.sample_count,
            dimension: desc.dimension.to_wgpu(),
            format: desc.format.to_wgpu(),
            usage: desc.usage.to_wgpu(),
            view_formats: &[],
        });
        let default_view = raw.create_view(&wgpu::TextureViewDescriptor {
            label: Some(desc.label),
            format: Some(desc.format.to_wgpu()),
            dimension: Some(desc.dimension.view_dimension()),
            aspect: if desc.format.is_depth() {
                wgpu::TextureAspect::DepthOnly
            } else {
                wgpu::TextureAspect::All
            },
            base_mip_level: 0,
            mip_level_count: Some(desc.mip_level_count),
            base_array_layer: 0,
            array_layer_count: None,
            usage: None,
        });
        Self {
            raw,
            default_view,
            format: desc.format,
            extent: desc.extent,
            dimension: desc.dimension,
            mip_level_count: desc.mip_level_count,
            device: device.clone(),
        }
    }

    /// Pixel format.
    pub fn format(&self) -> TextureFormat {
        self.format
    }

    /// Dimensions.
    pub fn extent(&self) -> Extent3d {
        self.extent
    }

    /// Texture dimension class.
    pub fn dimension(&self) -> TextureDimension {
        self.dimension
    }

    /// Mip-level count.
    pub fn mip_level_count(&self) -> u32 {
        self.mip_level_count
    }

    /// Borrow the default [`TextureView`] (all mips, full layer range).
    pub fn default_view(&self) -> TextureView<'_> {
        TextureView {
            raw: &self.default_view,
        }
    }

    /// Owning [`Device`] handle.
    pub fn device(&self) -> &Device {
        &self.device
    }

    /// Crate-internal access to the underlying `wgpu::Texture`.
    pub(crate) fn raw(&self) -> &wgpu::Texture {
        &self.raw
    }
}

/// Borrowed texture view. Returned by [`Texture::default_view`].
///
/// PR 2 keeps the view surface narrow: passes consume a `TextureView<'a>` as
/// render-target / sampled-texture binding. Custom subresource views land
/// alongside their consumer in PR 3+.
#[derive(Debug, Clone, Copy)]
pub struct TextureView<'a> {
    raw: &'a wgpu::TextureView,
}

impl<'a> TextureView<'a> {
    /// Crate-internal access to the underlying `wgpu::TextureView`.
    pub(crate) fn raw(&self) -> &wgpu::TextureView {
        self.raw
    }

    /// Crate-internal constructor used by the swapchain to wrap its
    /// per-frame surface view.
    pub(crate) fn from_raw(raw: &'a wgpu::TextureView) -> Self {
        Self { raw }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_bc_partitions_the_format_set() {
        let bc = [
            TextureFormat::Bc4RUnorm,
            TextureFormat::Bc5RgUnorm,
            TextureFormat::Bc6hRgbUfloat,
            TextureFormat::Bc7RgbaUnormSrgb,
            TextureFormat::Bc7RgbaUnorm,
        ];
        let non_bc = [
            TextureFormat::Rgba8UnormSrgb,
            TextureFormat::Rgba8Unorm,
            TextureFormat::Bgra8UnormSrgb,
            TextureFormat::Rgba16Float,
            TextureFormat::Depth32Float,
            TextureFormat::Depth32FloatStencil8,
            TextureFormat::Depth24PlusStencil8,
        ];
        for f in bc {
            assert!(f.is_bc(), "{f:?} should be BC");
            assert!(!f.is_depth(), "{f:?} should not be depth");
        }
        for f in non_bc {
            assert!(!f.is_bc(), "{f:?} should not be BC");
        }
    }

    #[test]
    fn is_depth_only_for_depth_formats() {
        let depth = [
            TextureFormat::Depth32Float,
            TextureFormat::Depth32FloatStencil8,
            TextureFormat::Depth24PlusStencil8,
        ];
        for f in depth {
            assert!(f.is_depth());
        }
        assert!(!TextureFormat::Rgba16Float.is_depth());
        assert!(!TextureFormat::Bc7RgbaUnormSrgb.is_depth());
    }

    #[test]
    fn extent_helper() {
        let e = Extent3d::new_2d(1280, 720);
        assert_eq!(e.width, 1280);
        assert_eq!(e.height, 720);
        assert_eq!(e.depth_or_array_layers, 1);
    }

    #[test]
    fn texture_usage_bitflags() {
        let u = TextureUsage::TEXTURE_BINDING | TextureUsage::COPY_DST;
        assert!(u.contains(TextureUsage::TEXTURE_BINDING));
        assert!(u.contains(TextureUsage::COPY_DST));
        assert!(!u.contains(TextureUsage::RENDER_ATTACHMENT));
        let mut v = TextureUsage::EMPTY;
        v |= TextureUsage::STORAGE_BINDING;
        assert!(v.contains(TextureUsage::STORAGE_BINDING));
    }

    #[test]
    fn albedo_2d_descriptor_defaults() {
        let d = TextureDesc::albedo_2d("hero", 256, 256, 9);
        assert_eq!(d.format, TextureFormat::Bc7RgbaUnormSrgb);
        assert_eq!(d.extent, Extent3d::new_2d(256, 256));
        assert_eq!(d.mip_level_count, 9);
        assert_eq!(d.dimension, TextureDimension::D2);
        assert!(d.usage.contains(TextureUsage::TEXTURE_BINDING));
        assert!(d.usage.contains(TextureUsage::COPY_DST));
    }
}
