//! Compilation targets for the Slang toolchain (ADR-037).
//!
//! Four canonical targets ride through the pipeline. Each maps to a
//! `slangc -target <name>` flag and a file extension under the
//! content-addressed pak. The `slangc` binary owns all the
//! source-language semantics; this module just enumerates the
//! shapes we feed in.

/// One of the four canonical Sliced Engine shader targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    /// Vulkan / SPIR-V binary (`-target spirv`). The renderer's
    /// canonical target on Linux and Windows-Vulkan.
    SpirV,
    /// WebGPU shading language source (`-target wgsl`). Browser /
    /// `wgpu` target.
    Wgsl,
    /// DirectX Intermediate Language binary (`-target dxil`).
    /// Windows-DX12 native target.
    Dxil,
    /// Metal Shading Language source (`-target metal`). macOS / iOS
    /// native target.
    Msl,
}

impl Target {
    /// The `-target <name>` flag value `slangc` recognises.
    pub fn slangc_flag(self) -> &'static str {
        match self {
            Self::SpirV => "spirv",
            Self::Wgsl => "wgsl",
            Self::Dxil => "dxil",
            Self::Msl => "metal",
        }
    }

    /// File extension we use under the pak for an artefact in this
    /// target. SPIR-V and DXIL are binary; WGSL and MSL are source.
    pub fn extension(self) -> &'static str {
        match self {
            Self::SpirV => "spv",
            Self::Wgsl => "wgsl",
            Self::Dxil => "dxil",
            Self::Msl => "metal",
        }
    }

    /// Whether the target produces a binary blob (true) or text
    /// source (false). The reflection oracle treats them
    /// identically — `bytes` is just bytes either way.
    pub fn is_binary(self) -> bool {
        matches!(self, Self::SpirV | Self::Dxil)
    }

    /// Iteration helper. Stable order for cross-arch goldens.
    pub fn all() -> &'static [Target] {
        &[Self::SpirV, Self::Wgsl, Self::Dxil, Self::Msl]
    }

    /// 1-byte tag used in the artefact bundle on-disk. Stable across
    /// versions: never re-number.
    pub fn tag(self) -> u8 {
        match self {
            Self::SpirV => 1,
            Self::Wgsl => 2,
            Self::Dxil => 3,
            Self::Msl => 4,
        }
    }

    /// Inverse of [`tag`](Self::tag).
    pub fn from_tag(b: u8) -> Option<Self> {
        match b {
            1 => Some(Self::SpirV),
            2 => Some(Self::Wgsl),
            3 => Some(Self::Dxil),
            4 => Some(Self::Msl),
            _ => None,
        }
    }
}

/// Shader pipeline stage. Used to route `slangc -stage <name>` and to
/// key reflection records.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Stage {
    /// Vertex shader.
    Vertex,
    /// Fragment / pixel shader.
    Fragment,
    /// Compute kernel.
    Compute,
}

impl Stage {
    /// The `-stage <name>` flag `slangc` recognises.
    pub fn slangc_flag(self) -> &'static str {
        match self {
            Self::Vertex => "vertex",
            Self::Fragment => "fragment",
            Self::Compute => "compute",
        }
    }

    /// 1-byte tag used in the artefact bundle on-disk.
    pub fn tag(self) -> u8 {
        match self {
            Self::Vertex => 1,
            Self::Fragment => 2,
            Self::Compute => 3,
        }
    }

    /// Inverse of [`tag`](Self::tag).
    pub fn from_tag(b: u8) -> Option<Self> {
        match b {
            1 => Some(Self::Vertex),
            2 => Some(Self::Fragment),
            3 => Some(Self::Compute),
            _ => None,
        }
    }
}
