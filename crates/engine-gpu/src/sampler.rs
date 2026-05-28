//! GPU sampler wrapper.
//!
//! Sampler descriptors are deterministically hashable (used by ADR-044 §5's
//! sampler interning). The owned enums map onto `wgpu::FilterMode` /
//! `wgpu::AddressMode`.

use crate::device::Device;

/// Texture-coordinate addressing mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AddressMode {
    /// Coordinates outside [0,1] wrap.
    Repeat,
    /// Coordinates outside [0,1] reflect.
    MirrorRepeat,
    /// Coordinates outside [0,1] clamp to the edge texel.
    ClampToEdge,
}

impl AddressMode {
    fn to_wgpu(self) -> wgpu::AddressMode {
        match self {
            AddressMode::Repeat => wgpu::AddressMode::Repeat,
            AddressMode::MirrorRepeat => wgpu::AddressMode::MirrorRepeat,
            AddressMode::ClampToEdge => wgpu::AddressMode::ClampToEdge,
        }
    }
}

/// Mip / min / mag filtering.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FilterMode {
    /// Nearest-neighbour. Used for pixel-art / data textures.
    Nearest,
    /// Bilinear / trilinear filtering. Used for everything else.
    Linear,
}

impl FilterMode {
    fn to_wgpu(self) -> wgpu::FilterMode {
        match self {
            FilterMode::Nearest => wgpu::FilterMode::Nearest,
            FilterMode::Linear => wgpu::FilterMode::Linear,
        }
    }
}

/// Sampler descriptor. Hashable so [`crate::BindlessHeap::intern_sampler`]
/// can dedupe by structural identity (ADR-044 §5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SamplerDesc {
    /// U-axis address mode.
    pub address_u: AddressMode,
    /// V-axis address mode.
    pub address_v: AddressMode,
    /// W-axis address mode (3D / cube; ignored for 2D).
    pub address_w: AddressMode,
    /// Magnification filter.
    pub mag_filter: FilterMode,
    /// Minification filter.
    pub min_filter: FilterMode,
    /// Mip filter.
    pub mipmap_filter: FilterMode,
    /// Maximum anisotropy. 1 = isotropic. 4 / 8 / 16 are the typical
    /// per-tier upgrades.
    pub anisotropy: u16,
    /// Enable comparison sampling. When `true`, the sampler is a
    /// PCF / shadow-comparison sampler (wgpu sets the
    /// [`wgpu::SamplerDescriptor::compare`] field, surfacing the
    /// `sampler_comparison` shader binding type). Reverse-Z passes
    /// use the `Greater` comparator the engine wires in
    /// [`Sampler::new`]; the descriptor's bool surface keeps the
    /// hashable structure of `SamplerDesc` minimal while still
    /// expressing the binding-type difference.
    pub comparison: bool,
}

impl SamplerDesc {
    /// Default linear-trilinear repeat sampler — covers most material
    /// uses.
    pub const fn linear_repeat() -> Self {
        Self {
            address_u: AddressMode::Repeat,
            address_v: AddressMode::Repeat,
            address_w: AddressMode::Repeat,
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            mipmap_filter: FilterMode::Linear,
            anisotropy: 1,
            comparison: false,
        }
    }

    /// Nearest-neighbour clamp — pixel-art / data textures.
    pub const fn nearest_clamp() -> Self {
        Self {
            address_u: AddressMode::ClampToEdge,
            address_v: AddressMode::ClampToEdge,
            address_w: AddressMode::ClampToEdge,
            mag_filter: FilterMode::Nearest,
            min_filter: FilterMode::Nearest,
            mipmap_filter: FilterMode::Nearest,
            anisotropy: 1,
            comparison: false,
        }
    }

    /// Shadow PCF comparison sampler (clamp address, linear filter,
    /// reverse-Z `Greater` compare). Used by the CSM atlas sampler in
    /// [`crate::LightingAccumulationPass`] (ADR-040).
    pub const fn shadow_pcf() -> Self {
        Self {
            address_u: AddressMode::ClampToEdge,
            address_v: AddressMode::ClampToEdge,
            address_w: AddressMode::ClampToEdge,
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            mipmap_filter: FilterMode::Nearest,
            anisotropy: 1,
            comparison: true,
        }
    }
}

impl Default for SamplerDesc {
    fn default() -> Self {
        Self::linear_repeat()
    }
}

/// Owned GPU sampler.
#[derive(Debug)]
pub struct Sampler {
    #[allow(dead_code, reason = "consumed by PR 3 bind-group entries")]
    raw: wgpu::Sampler,
    desc: SamplerDesc,
}

impl Sampler {
    /// Create a sampler through the device.
    pub fn new(device: &Device, desc: SamplerDesc) -> Self {
        // wgpu 29 split the mipmap filter into its own `MipmapFilterMode`
        // enum (Nearest / Linear); the engine-facing `FilterMode` is
        // re-used for both axes via a small projection here.
        let mipmap = match desc.mipmap_filter {
            FilterMode::Nearest => wgpu::MipmapFilterMode::Nearest,
            FilterMode::Linear => wgpu::MipmapFilterMode::Linear,
        };
        // Reverse-Z (ADR-040 §3) shadow passes use Greater; the
        // SamplerDesc.comparison bool elides the compare-function
        // surface from the hashable descriptor (every shadow sampler
        // in the engine is reverse-Z).
        let compare = if desc.comparison {
            Some(wgpu::CompareFunction::Greater)
        } else {
            None
        };
        let raw = device.raw().create_sampler(&wgpu::SamplerDescriptor {
            label: None,
            address_mode_u: desc.address_u.to_wgpu(),
            address_mode_v: desc.address_v.to_wgpu(),
            address_mode_w: desc.address_w.to_wgpu(),
            mag_filter: desc.mag_filter.to_wgpu(),
            min_filter: desc.min_filter.to_wgpu(),
            mipmap_filter: mipmap,
            lod_min_clamp: 0.0,
            lod_max_clamp: 32.0,
            compare,
            anisotropy_clamp: desc.anisotropy.max(1),
            border_color: None,
        });
        Self { raw, desc }
    }

    /// Descriptor that produced this sampler.
    pub fn desc(&self) -> SamplerDesc {
        self.desc
    }

    /// Crate-internal access to the underlying `wgpu::Sampler`. Consumed
    /// by PR 3 bind-group construction.
    #[allow(dead_code, reason = "consumed by PR 3 bind-group entries")]
    pub(crate) fn raw(&self) -> &wgpu::Sampler {
        &self.raw
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_repeat_defaults() {
        let s = SamplerDesc::linear_repeat();
        assert_eq!(s.address_u, AddressMode::Repeat);
        assert_eq!(s.address_v, AddressMode::Repeat);
        assert_eq!(s.address_w, AddressMode::Repeat);
        assert_eq!(s.mag_filter, FilterMode::Linear);
        assert_eq!(s.min_filter, FilterMode::Linear);
        assert_eq!(s.mipmap_filter, FilterMode::Linear);
        assert_eq!(s.anisotropy, 1);
    }

    #[test]
    fn nearest_clamp_defaults() {
        let s = SamplerDesc::nearest_clamp();
        assert_eq!(s.address_u, AddressMode::ClampToEdge);
        assert_eq!(s.mag_filter, FilterMode::Nearest);
        assert_eq!(s.mipmap_filter, FilterMode::Nearest);
    }

    #[test]
    fn descriptors_are_hashable_for_interning() {
        // Sanity-check the trait wiring ADR-044's intern_sampler relies on.
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(SamplerDesc::linear_repeat());
        set.insert(SamplerDesc::linear_repeat());
        set.insert(SamplerDesc::nearest_clamp());
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn default_is_linear_repeat() {
        assert_eq!(SamplerDesc::default(), SamplerDesc::linear_repeat());
    }
}
