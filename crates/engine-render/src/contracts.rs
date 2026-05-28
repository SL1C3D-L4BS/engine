//! GPU pass contract types — Rust representations of the descriptor
//! layouts, push-constant payloads, and SSBO record shapes the Track-A
//! deferred pipeline binds (ADR-064 + ADR-065).
//!
//! The 11 Phase-5 pass stubs ship as `record()` no-ops; their wgpu
//! draw/dispatch bodies arrive when the self-hosted GPU runner
//! lets us validate pixel parity (Phase 6 PR 3.5 + PR 4.5). Until
//! then, this module pins the *contracts* both sides of the boundary
//! must agree on:
//!
//! - Push-constant payload (`PushConstants`) shared across geometry
//!   passes.
//! - Per-pass uniform-buffer layouts (`CsmUniforms`,
//!   `ClusterUniforms`, `SsaoUniforms`, `TaaUniforms`,
//!   `BloomUniforms`, `TonemapUniforms`, `IblUniforms`).
//! - SSBO record types for the cluster grid (`ClusterCell`,
//!   `LightRecord`) and the IBL probe set (`IblProbeRecord`).
//! - MRT format constants for the G-buffer + depth target.
//! - Cross-checked geometry constants
//!   (`CLUSTER_TILES_{X,Y,Z}`, `CSM_CASCADES`, `MAX_LIGHTS_PER_CLUSTER`,
//!   `MAX_IBL_PROBES`, `BRDF_LUT_DIM`) that mirror the CPU oracle in
//!   `testbed/engine-raster/src/{cluster,shadow,ibl}.rs`. The integration
//!   test `tests/contract_layouts.rs` asserts size/alignment so a
//!   future shader edit can't desync from the layout silently.
//!
//! ## ADR cross-reference
//!
//! - `PushConstants` — ADR-063 §5 + ADR-064 §2.
//! - `CsmUniforms` — ADR-064 §4.
//! - `ClusterUniforms` + `ClusterCell` + `LightRecord` — ADR-064 §5.
//! - G-buffer formats — ADR-064 §3.
//! - `SsaoUniforms`, `TaaUniforms`, `BloomUniforms`, `TonemapUniforms`,
//!   `IblUniforms`, `IblProbeRecord` — ADR-065 §1-§6.

use engine_gpu::TextureFormat;

// ---------------------------------------------------------------------------
// Geometry / lighting (ADR-064)
// ---------------------------------------------------------------------------

/// Per-draw push constants. 64 bytes — the maximum portable size
/// across Vulkan / D3D12 / Metal / WebGPU (ADR-063 §5).
///
/// `repr(C)` so the field layout matches WGSL's `@push_constant` block
/// declared in the shader sources. Future shader edits that add fields
/// must consume the `reserved` slot first; bumping past 64 B requires
/// an ADR amendment.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PushConstants {
    /// 3×4 affine model transform (object → world).
    pub model_xform: [f32; 12],
    /// Index into the bindless EMAT material pool (ADR-044).
    pub material_index: u32,
    /// Instance id (for indirect draws).
    pub instance_id: u32,
    /// Per-draw bitset (selectable shadow cascades, alpha-mode hint).
    pub flags: u32,
    /// Reserved for v2 expansion; zero in v1.
    pub reserved: u32,
}

const _: () = assert!(core::mem::size_of::<PushConstants>() == 64);

/// CSM uniform block bound to Group 1 of the `CsmShadowPass`. 256 B.
/// Carries the four cascade view-projection matrices plus the
/// view-space split bounds (ADR-064 §4).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CsmUniforms {
    /// One column-major mat4x4 per cascade.
    pub cascade_view_projection: [[f32; 16]; CSM_CASCADES_CONTRACT],
    /// Far-plane view-space depth for each cascade.
    pub cascade_split_far: [f32; CSM_CASCADES_CONTRACT],
    /// PCF kernel radius in atlas texels.
    pub filter_radius_px: f32,
    /// Constant depth bias.
    pub bias_constant: f32,
    /// Slope-scaled depth bias.
    pub bias_slope: f32,
}

const _: () = assert!(core::mem::size_of::<CsmUniforms>() == 16 * 16 + 4 * 4 + 4 * 3);

/// Cluster light uniforms bound to Group 1 of the `ClusterLightPass`.
/// Carries the cluster-grid parameters + the view-frustum projection
/// (ADR-064 §5).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClusterUniforms {
    /// Inverse view-projection — light-space frustum reconstruction.
    pub inv_view_projection: [f32; 16],
    /// Number of active lights in the light SSBO.
    pub light_count: u32,
    /// Cluster grid dimensions (x, y, z) — should equal the
    /// CLUSTER_TILES_{X,Y,Z} constants but bundled here for shader
    /// access.
    pub grid_dim: [u32; 3],
    /// Logarithmic-Z near plane.
    pub z_near: f32,
    /// Logarithmic-Z far plane.
    pub z_far: f32,
    /// Reserved (pad to 16 B alignment within the UBO).
    pub reserved: [f32; 2],
}

/// One cluster-grid cell — light count + offset into a parallel index
/// SSBO (ADR-064 §5).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClusterCell {
    /// Offset into the light_indices SSBO where this cell's list starts.
    pub light_offset: u32,
    /// Number of lights assigned to this cell.
    pub light_count: u32,
}

const _: () = assert!(core::mem::size_of::<ClusterCell>() == 8);

/// One light record in the LightData SSBO (ADR-064 §5).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightRecord {
    /// xyz = world-space position, w = radius.
    pub position_radius: [f32; 4],
    /// rgb = colour, a = intensity (cd/m²).
    pub color_intensity: [f32; 4],
    /// xyz = direction (spot / sun), w = packed bits.
    pub direction: [f32; 4],
    /// x = inner-cone cos, y = outer-cone cos, z = falloff exponent,
    /// w = light type tag (0 = point, 1 = spot, 2 = directional).
    pub params: [f32; 4],
}

const _: () = assert!(core::mem::size_of::<LightRecord>() == 64);

// ---------------------------------------------------------------------------
// Cluster-grid contract constants (mirror testbed/engine-raster oracle)
// ---------------------------------------------------------------------------

/// X-axis cluster tile count (ADR-043 + testbed/engine-raster `cluster::CLUSTER_TILES_X`).
pub const CLUSTER_TILES_X: u32 = 16;
/// Y-axis cluster tile count.
pub const CLUSTER_TILES_Y: u32 = 9;
/// Z-axis cluster slice count.
pub const CLUSTER_TILES_Z: u32 = 24;
/// Total cluster cells.
pub const CLUSTER_CELL_COUNT: u32 = CLUSTER_TILES_X * CLUSTER_TILES_Y * CLUSTER_TILES_Z;
/// Cap on lights per cluster (ADR-043).
pub const MAX_LIGHTS_PER_CLUSTER: u32 = 32;
/// Cap on total light count in `LightRecord` SSBO (ADR-064 §5).
pub const MAX_TOTAL_LIGHTS: u32 = 256;

// ---------------------------------------------------------------------------
// CSM contract constants (mirror testbed/engine-raster oracle)
// ---------------------------------------------------------------------------

/// Cascade count (ADR-040 + testbed/engine-raster `shadow::CSM_CASCADES`).
pub const CSM_CASCADES_CONTRACT: usize = 4;
/// Atlas dimension per axis (ADR-040 + testbed/engine-raster `shadow::ATLAS_DIM`).
pub const CSM_ATLAS_DIM: u32 = 4096;
/// Sub-quadrant dimension.
pub const CSM_CASCADE_DIM: u32 = CSM_ATLAS_DIM / 2;
/// Practical-split blend factor (ADR-040 §1).
pub const CSM_PRACTICAL_SPLIT_LAMBDA: f32 = 0.6;

// ---------------------------------------------------------------------------
// MRT format contract (ADR-064 §3)
// ---------------------------------------------------------------------------

/// G-buffer slot 0: albedo (sRGB) + roughness (linear A).
pub const GBUFFER_ALBEDO_ROUGHNESS_FORMAT: TextureFormat = TextureFormat::Rgba8UnormSrgb;
/// G-buffer slot 1: normal (xyz) + metallic (w).
pub const GBUFFER_NORMAL_METALLIC_FORMAT: TextureFormat = TextureFormat::Rgba16Float;
/// G-buffer slot 2: motion (xy) + view-depth (z) + ID (w).
pub const GBUFFER_MOTION_DEPTH_FORMAT: TextureFormat = TextureFormat::Rgba16Float;
/// Depth attachment format (reverse-Z, ADR-040 §3).
pub const DEPTH_BUFFER_FORMAT: TextureFormat = TextureFormat::Depth32Float;
/// Lighting accumulation target.
pub const LIT_COLOR_FORMAT: TextureFormat = TextureFormat::Rgba16Float;

// ---------------------------------------------------------------------------
// Compute workgroup-size contracts
// ---------------------------------------------------------------------------

/// CullPass workgroup size (one thread per instance batch, ADR-064 §7).
pub const CULL_WORKGROUP_SIZE: [u32; 3] = [64, 1, 1];
/// ClusterLightPass workgroup size (one thread per cluster cell).
pub const CLUSTER_ASSIGN_WORKGROUP_SIZE: [u32; 3] = [16, 9, 1];

// ---------------------------------------------------------------------------
// Post-FX (ADR-065)
// ---------------------------------------------------------------------------

/// 8-tap Fibonacci kernel SSAO uniforms (ADR-065 §1).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SsaoUniforms {
    /// Inverse projection — depth → view-space.
    pub inverse_projection: [f32; 16],
    /// 8 Fibonacci kernel samples (`.xyz` direction, `.w` weight).
    pub kernel: [[f32; 4]; SSAO_KERNEL_TAPS],
    /// World-space sample radius.
    pub radius: f32,
    /// Bias to avoid self-occlusion.
    pub bias: f32,
    /// Intensity multiplier.
    pub intensity: f32,
    /// Reserved (pad).
    pub reserved: f32,
}

/// Fibonacci kernel tap count (ADR-065 §1 + CPU oracle).
pub const SSAO_KERNEL_TAPS: usize = 8;

/// TAA uniforms bound to Group 1 (ADR-065 §4).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TaaUniforms {
    /// Previous frame's view-projection matrix (for reprojection).
    pub prev_view_projection: [f32; 16],
    /// Halton(2,3) jitter for the current frame.
    pub jitter_current: [f32; 2],
    /// Halton(2,3) jitter for the previous frame.
    pub jitter_prev: [f32; 2],
    /// Blend coefficient (α ∈ [0.05, 0.5]).
    pub blend_alpha: f32,
    /// Disocclusion-mask depth-ratio threshold (ADR-065 §4).
    pub disocclusion_threshold: f32,
    /// Reserved (pad to 16 B alignment).
    pub reserved: [f32; 2],
}

/// Bloom uniforms (ADR-065 §5).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BloomUniforms {
    /// Soft-knee threshold (HDR luminance).
    pub threshold: f32,
    /// Soft-knee blend width.
    pub soft_knee: f32,
    /// Output intensity multiplier.
    pub intensity: f32,
    /// Pad to 16 B alignment.
    pub reserved: f32,
}

/// Bloom mip chain depth (ADR-065 §5).
pub const BLOOM_MIP_LEVELS: u32 = 5;

/// Tonemap uniforms (ADR-065 §6).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TonemapUniforms {
    /// Exposure multiplier in linear space.
    pub exposure: f32,
    /// Bloom contribution mix.
    pub bloom_mix: f32,
    /// ACES white-point.
    pub white_point: f32,
    /// Pad to 16 B alignment.
    pub reserved: f32,
}

/// IBL uniforms (ADR-065 §2).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IblUniforms {
    /// Inverse view-projection (world-space sample direction).
    pub inv_view_projection: [f32; 16],
    /// Number of probes in the IBL probe SSBO.
    pub probe_count: u32,
    /// Cell-side length in metres (default 4 m, ADR-041 §1).
    pub cell_size_m: f32,
    /// Pad to 16 B alignment.
    pub reserved: [f32; 2],
}

/// One IBL L2-SH probe in the SSBO (ADR-065 §2 — 9 RGB SH coefficients
/// packed as 9 × `vec4<f32>` for alignment).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IblProbeRecord {
    /// Hashed cell key (xyz integer cell coordinates).
    pub cell_key: [i32; 3],
    /// Pad to vec4 alignment.
    pub pad: u32,
    /// 9 L2-SH coefficients (`.xyz` RGB; `.w` ignored).
    pub sh_coeffs: [[f32; 4]; 9],
}

const _: () = assert!(core::mem::size_of::<IblProbeRecord>() == 16 + 9 * 16);

/// IBL probe set caps (ADR-041 §1 + CPU oracle `engine_raster::ibl`).
pub const MAX_IBL_PROBES: u32 = 128;
/// Default IBL cell size in metres (ADR-041 §1).
pub const DEFAULT_IBL_CELL_SIZE_M: f32 = 4.0;
/// BRDF LUT square dimension (ADR-065 §3).
pub const BRDF_LUT_DIM: u32 = 512;
/// SSAO target resolution divisor (half-res, ADR-065 §1).
pub const SSAO_RESOLUTION_DIVISOR: u32 = 2;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_constants_is_64_bytes() {
        assert_eq!(core::mem::size_of::<PushConstants>(), 64);
    }

    #[test]
    fn cluster_cell_is_8_bytes() {
        assert_eq!(core::mem::size_of::<ClusterCell>(), 8);
    }

    #[test]
    fn light_record_is_64_bytes() {
        assert_eq!(core::mem::size_of::<LightRecord>(), 64);
    }

    #[test]
    fn ibl_probe_record_matches_layout() {
        assert_eq!(core::mem::size_of::<IblProbeRecord>(), 160);
    }

    #[test]
    fn cluster_cell_count_matches_grid() {
        assert_eq!(
            CLUSTER_CELL_COUNT,
            CLUSTER_TILES_X * CLUSTER_TILES_Y * CLUSTER_TILES_Z
        );
        assert_eq!(CLUSTER_CELL_COUNT, 16 * 9 * 24);
    }

    #[test]
    fn csm_cascade_dim_is_half_atlas() {
        assert_eq!(CSM_CASCADE_DIM, CSM_ATLAS_DIM / 2);
        assert_eq!(CSM_CASCADE_DIM, 2048);
    }
}
