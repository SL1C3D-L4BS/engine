//! Resource type tags for the deferred render graph (PR 3).
//!
//! Every graph-managed resource is a zero-sized type implementing
//! [`crate::render_graph::ResourceType`]. The tag carries:
//!
//! - `KIND` — which allocator family (buffer vs. texture vs. sampler
//!   vs. swapchain) the [`engine_gpu`] backend should serve from.
//! - `NAME` — the human-readable label that appears in telemetry spans
//!   (`SPAN("draw.opaque", Subsystem::Render)` per spec §VII.4).
//!
//! The phantom-type machinery in [`crate::Resource`] keeps a pass that
//! declared a [`ShadowAtlas`] handle from accidentally binding a
//! [`ClusterCells`] slot. PR 3's five passes name each tag explicitly
//! in their `reads()` / `writes()` callbacks; downstream PRs add new
//! tags here as they introduce new resources.

use crate::render_graph::{ResourceKind, ResourceType};

/// Per-frame render queue: opaque draw commands in submission order.
/// Produced by the front-end render-graph builder (the future
/// `geom.feed` pass in Track-A); consumed by the cull pass.
pub struct RenderQueue;
impl ResourceType for RenderQueue {
    const KIND: ResourceKind = ResourceKind::Buffer;
    const NAME: &'static str = "render_queue";
}

/// Per-instance shadow-caster subset of the render queue. Produced by
/// the front-end; consumed by the CSM shadow pass.
pub struct ShadowCasters;
impl ResourceType for ShadowCasters {
    const KIND: ResourceKind = ResourceKind::Buffer;
    const NAME: &'static str = "shadow_casters";
}

/// Indirect-draw command buffer produced by the cull pass and consumed
/// by the geometry pass. Each cluster's surviving instances appear as
/// a `DrawIndexedIndirect` record.
pub struct IndirectDrawBuffer;
impl ResourceType for IndirectDrawBuffer {
    const KIND: ResourceKind = ResourceKind::Buffer;
    const NAME: &'static str = "indirect_draws";
}

/// Per-light SSBO. Mirrors the GPU layout described in ADR-043 §3
/// (`GpuLight`). Produced by the front-end; consumed by the cluster
/// pass and the lighting accumulation pass.
pub struct LightSsbo;
impl ResourceType for LightSsbo {
    const KIND: ResourceKind = ResourceKind::Buffer;
    const NAME: &'static str = "lights";
}

/// Cluster grid SSBO — 16 × 9 × 24 = 3 456 `ClusterCell`s, ~270 KiB.
/// Produced by the cluster pass; consumed by the lighting pass. ADR-043.
pub struct ClusterCells;
impl ResourceType for ClusterCells {
    const KIND: ResourceKind = ResourceKind::Buffer;
    const NAME: &'static str = "cluster_cells";
}

/// G-buffer #1: albedo (RGB) + roughness (A). ADR-049 / spec §IV.4.A.
pub struct GBufferAlbedoRoughness;
impl ResourceType for GBufferAlbedoRoughness {
    const KIND: ResourceKind = ResourceKind::Texture;
    const NAME: &'static str = "gbuffer.albedo_roughness";
}

/// G-buffer #2: octahedral-encoded normals (RG) + metallic (B). The
/// per-instance ambient occlusion factor lands in the A channel.
pub struct GBufferNormalMetallic;
impl ResourceType for GBufferNormalMetallic {
    const KIND: ResourceKind = ResourceKind::Texture;
    const NAME: &'static str = "gbuffer.normal_metallic";
}

/// G-buffer #3: motion vectors (RG) + view-space depth (B). TAA in
/// PR 4 reads the RG channels; the cluster pass and the cluster
/// fragment lookup read B.
pub struct GBufferMotionDepth;
impl ResourceType for GBufferMotionDepth {
    const KIND: ResourceKind = ResourceKind::Texture;
    const NAME: &'static str = "gbuffer.motion_depth";
}

/// Hardware depth buffer used by the geometry pass and read-only by
/// the lighting accumulation pass. D32F per spec §IV.4.A.
pub struct DepthBuffer;
impl ResourceType for DepthBuffer {
    const KIND: ResourceKind = ResourceKind::Texture;
    const NAME: &'static str = "depth";
}

/// 4096² D32F cascade shadow atlas (ADR-040 §3). Produced by the CSM
/// shadow pass; consumed by the lighting pass.
pub struct ShadowAtlas;
impl ResourceType for ShadowAtlas {
    const KIND: ResourceKind = ResourceKind::Texture;
    const NAME: &'static str = "shadow_atlas";
}

/// HDR linear-space output the lighting pass writes into. Read by the
/// future post-FX chain (PR 4) and tonemap.
pub struct LitColor;
impl ResourceType for LitColor {
    const KIND: ResourceKind = ResourceKind::Texture;
    const NAME: &'static str = "lit_color";
}

/// IBL probe set buffer (L2 SH coefficients, ADR-041 §5). Produced by
/// the offline bake tool (`engine-ibl-bake`, deferred to Phase 5 follow-
/// up) and uploaded once per scene; consumed by [`IblPass`].
pub struct IblProbeSet;
impl ResourceType for IblProbeSet {
    const KIND: ResourceKind = ResourceKind::Buffer;
    const NAME: &'static str = "ibl_probes";
}

/// 2D BRDF lookup texture for the Karis split-sum specular term
/// (ADR-041 §4). 512×512 RG16F, ships in the pak as a single asset.
pub struct BrdfLut;
impl ResourceType for BrdfLut {
    const KIND: ResourceKind = ResourceKind::Texture;
    const NAME: &'static str = "brdf_lut";
}

/// Screen-space ambient-occlusion attachment (R8). Produced by
/// [`SsaoPass`]; consumed by the [`crate::LightingAccumulationPass`] and
/// the bloom/tonemap chain.
pub struct SsaoTexture;
impl ResourceType for SsaoTexture {
    const KIND: ResourceKind = ResourceKind::Texture;
    const NAME: &'static str = "ssao";
}

/// Previous-frame TAA history (RGBA16F at native render resolution).
/// Double-buffered by the resource pool (ADR-042 §6); the pass reads
/// the previous slot and writes the current slot.
pub struct TaaHistory;
impl ResourceType for TaaHistory {
    const KIND: ResourceKind = ResourceKind::Texture;
    const NAME: &'static str = "taa_history";
}

/// TAA-resolved HDR target. Canonical input to the
/// [`UpscalerProvider`](crate) trait (ADR-005) and to the bloom +
/// tonemap stages.
pub struct TaaResolvedColor;
impl ResourceType for TaaResolvedColor {
    const KIND: ResourceKind = ResourceKind::Texture;
    const NAME: &'static str = "taa_resolved";
}

/// Bloom layer — low-frequency bright-pass blur (ADR-042 §spec post
/// chain). Produced by [`BloomPass`]; composited by [`TonemapPass`].
pub struct BloomTexture;
impl ResourceType for BloomTexture {
    const KIND: ResourceKind = ResourceKind::Texture;
    const NAME: &'static str = "bloom";
}

/// Final LDR tonemapped output. The compositor / swapchain consumer
/// reads this; under Track A it is also the upscaler input when
/// rendering below native resolution.
pub struct TonemappedColor;
impl ResourceType for TonemappedColor {
    const KIND: ResourceKind = ResourceKind::Texture;
    const NAME: &'static str = "tonemapped";
}
