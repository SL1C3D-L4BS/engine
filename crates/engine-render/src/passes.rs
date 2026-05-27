//! Phase 5 PR 3 + PR 4 + PR 7 — ten deferred render-graph passes.
//!
//! All implementations name [`engine_gpu`] types; none names `wgpu`
//! directly (ADR-049). Each pass owns a lazily-built pipeline that
//! the GPU-side `record()` body binds; PR 7 lands the construction
//! surface so PR 8 can populate resource bindings + pixel-parity
//! fixtures without further pass-struct churn.
//!
//! Pass scheduling order produced by `RenderGraph::compile()`:
//!
//! 1. [`CullPass`] — `RenderQueue` → `IndirectDrawBuffer`.
//! 2. [`CsmShadowPass`] — `ShadowCasters` → `ShadowAtlas`.
//! 3. [`ClusterLightPass`] — `LightSsbo` → `ClusterCells`.
//! 4. [`GBufferPass`] — `IndirectDrawBuffer` → MRT G-buffer + depth.
//! 5. [`SsaoPass`] — depth + normals → `SsaoTexture`.
//! 6. [`IblPass`] — probes + BRDF LUT + G-buffer → `LitColor` (IBL pre-fill).
//! 7. [`LightingAccumulationPass`] — full direct-light Cook-Torrance pass.
//! 8. [`TaaPass`] — `LitColor` + `TaaHistory` + motion → `TaaResolvedColor`.
//! 9. [`BloomPass`] — `TaaResolvedColor` → `BloomTexture`.
//! 10. [`TonemapPass`] — TAA + bloom → `TonemappedColor`.
//!
//! ## Upscale-path variant (PR 5, ADR-005 + ADR-053)
//!
//! When the renderer is upscaling internal-resolution output to a
//! larger display, [`UpscalePass`] slots between [`TaaPass`] and
//! [`TonemapPass`]: `TaaResolvedColor` → `UpscaledColor` → tonemap.
//! Bloom still extracts from the TAA-resolved (pre-upscale) buffer to
//! preserve energy; tonemap composites the bloom layer over the
//! upscaled HDR. Selection between the no-upscale and upscale variants
//! is a graph-builder decision; both variants compile and execute
//! through the same [`RenderGraph`] API.
//!
//! ## PR 7 wiring scope
//!
//! Each pass owns a [`std::sync::OnceLock`] holding the compiled
//! pipeline; the first `record()` call against a real
//! [`crate::render_graph::GpuFrameContext`] lazy-builds the pipeline
//! from the embedded WGSL constants in [`crate::shaders`] via
//! [`crate::shader::wgsl_artefact_set`]. Compute passes also issue
//! their compute-pass scope (begin → set_pipeline → dispatch) with
//! placeholder dispatch dimensions; PR 8 wires real resource lookups +
//! bind groups + dispatch counts. Render passes (depth-only,
//! MRT G-buffer, full-screen lighting) lazy-init the pipeline but
//! defer `begin_render_pass` to PR 8 because the current
//! [`engine_gpu`] surface requires attachment [`engine_gpu::TextureView`]s
//! that the render graph's transient resource pool doesn't yet
//! resolve.

use std::sync::OnceLock;

use engine_gpu::{ColorTargetState, ComputePipeline, DepthStencilState, Device, RenderPipeline};
use engine_shader::Stage;

use crate::contracts::{
    DEPTH_BUFFER_FORMAT, GBUFFER_ALBEDO_ROUGHNESS_FORMAT, GBUFFER_MOTION_DEPTH_FORMAT,
    GBUFFER_NORMAL_METALLIC_FORMAT, LIT_COLOR_FORMAT,
};
use crate::render_graph::{Pass, PassContext, ResourceId, ResourceSet, Track};
use crate::shader::{
    ComputePipelineHelperDesc, RenderPipelineHelperDesc, build_compute_pipeline,
    build_render_pipeline, wgsl_artefact_set,
};
use crate::shaders::{
    BLOOM_WGSL, CLUSTER_ASSIGN_WGSL, CSM_SHADOW_WGSL, CULL_WGSL, GBUFFER_WGSL, IBL_EVALUATE_WGSL,
    LIGHTING_WGSL, SSAO_WGSL, TAA_RESOLVE_WGSL, TONEMAP_WGSL,
};

// =============================================================================
// Pipeline-builder helpers (one per pass / per entry point).
// =============================================================================

pub(crate) fn build_cull_pipeline(device: &Device) -> ComputePipeline {
    let cs = wgsl_artefact_set(Stage::Compute, "cs_main", CULL_WGSL);
    build_compute_pipeline(
        device,
        &ComputePipelineHelperDesc {
            label: "cull",
            compute: &cs,
            entry: "cs_main",
        },
    )
    .expect("cull pipeline build")
}

pub(crate) fn build_csm_shadow_pipeline(device: &Device) -> RenderPipeline {
    let vs = wgsl_artefact_set(Stage::Vertex, "vs_main", CSM_SHADOW_WGSL);
    build_render_pipeline(
        device,
        &RenderPipelineHelperDesc {
            label: "shadow",
            vertex: &vs,
            vertex_entry: "vs_main",
            vertex_buffers: &[],
            fragment: None,
            fragment_entry: "",
            color_targets: &[],
            depth_stencil: Some(DepthStencilState {
                format: DEPTH_BUFFER_FORMAT,
                depth_write_enabled: true,
            }),
        },
    )
    .expect("csm_shadow pipeline build")
}

pub(crate) fn build_cluster_assign_pipeline(device: &Device) -> ComputePipeline {
    let cs = wgsl_artefact_set(Stage::Compute, "cs_main", CLUSTER_ASSIGN_WGSL);
    build_compute_pipeline(
        device,
        &ComputePipelineHelperDesc {
            label: "light.cluster",
            compute: &cs,
            entry: "cs_main",
        },
    )
    .expect("cluster_assign pipeline build")
}

pub(crate) fn build_gbuffer_pipeline(device: &Device) -> RenderPipeline {
    let vs = wgsl_artefact_set(Stage::Vertex, "vs_main", GBUFFER_WGSL);
    let fs = wgsl_artefact_set(Stage::Fragment, "fs_main", GBUFFER_WGSL);
    build_render_pipeline(
        device,
        &RenderPipelineHelperDesc {
            label: "gbuffer",
            vertex: &vs,
            vertex_entry: "vs_main",
            vertex_buffers: &[],
            fragment: Some(&fs),
            fragment_entry: "fs_main",
            color_targets: &[
                ColorTargetState {
                    format: GBUFFER_ALBEDO_ROUGHNESS_FORMAT,
                },
                ColorTargetState {
                    format: GBUFFER_NORMAL_METALLIC_FORMAT,
                },
                ColorTargetState {
                    format: GBUFFER_MOTION_DEPTH_FORMAT,
                },
            ],
            depth_stencil: Some(DepthStencilState {
                format: DEPTH_BUFFER_FORMAT,
                depth_write_enabled: true,
            }),
        },
    )
    .expect("gbuffer pipeline build")
}

pub(crate) fn build_ssao_pipeline(device: &Device) -> ComputePipeline {
    let cs = wgsl_artefact_set(Stage::Compute, "cs_main", SSAO_WGSL);
    build_compute_pipeline(
        device,
        &ComputePipelineHelperDesc {
            label: "post.fx.ssao",
            compute: &cs,
            entry: "cs_main",
        },
    )
    .expect("ssao pipeline build")
}

pub(crate) fn build_ibl_evaluate_pipeline(device: &Device) -> ComputePipeline {
    let cs = wgsl_artefact_set(Stage::Compute, "cs_main", IBL_EVALUATE_WGSL);
    build_compute_pipeline(
        device,
        &ComputePipelineHelperDesc {
            label: "draw.opaque.ibl",
            compute: &cs,
            entry: "cs_main",
        },
    )
    .expect("ibl_evaluate pipeline build")
}

pub(crate) fn build_lighting_pipeline(device: &Device) -> RenderPipeline {
    let vs = wgsl_artefact_set(Stage::Vertex, "vs_main", LIGHTING_WGSL);
    let fs = wgsl_artefact_set(Stage::Fragment, "fs_main", LIGHTING_WGSL);
    build_render_pipeline(
        device,
        &RenderPipelineHelperDesc {
            label: "draw.opaque.2",
            vertex: &vs,
            vertex_entry: "vs_main",
            vertex_buffers: &[],
            fragment: Some(&fs),
            fragment_entry: "fs_main",
            color_targets: &[ColorTargetState {
                format: LIT_COLOR_FORMAT,
            }],
            depth_stencil: None,
        },
    )
    .expect("lighting pipeline build")
}

pub(crate) fn build_taa_resolve_pipeline(device: &Device) -> ComputePipeline {
    let cs = wgsl_artefact_set(Stage::Compute, "cs_main", TAA_RESOLVE_WGSL);
    build_compute_pipeline(
        device,
        &ComputePipelineHelperDesc {
            label: "post.fx.taa",
            compute: &cs,
            entry: "cs_main",
        },
    )
    .expect("taa_resolve pipeline build")
}

pub(crate) fn build_bloom_extract_pipeline(device: &Device) -> ComputePipeline {
    let cs = wgsl_artefact_set(Stage::Compute, "cs_extract", BLOOM_WGSL);
    build_compute_pipeline(
        device,
        &ComputePipelineHelperDesc {
            label: "post.fx.bloom.extract",
            compute: &cs,
            entry: "cs_extract",
        },
    )
    .expect("bloom extract pipeline build")
}

pub(crate) fn build_bloom_downsample_pipeline(device: &Device) -> ComputePipeline {
    let cs = wgsl_artefact_set(Stage::Compute, "cs_downsample", BLOOM_WGSL);
    build_compute_pipeline(
        device,
        &ComputePipelineHelperDesc {
            label: "post.fx.bloom.downsample",
            compute: &cs,
            entry: "cs_downsample",
        },
    )
    .expect("bloom downsample pipeline build")
}

pub(crate) fn build_bloom_upsample_pipeline(device: &Device) -> ComputePipeline {
    let cs = wgsl_artefact_set(Stage::Compute, "cs_upsample", BLOOM_WGSL);
    build_compute_pipeline(
        device,
        &ComputePipelineHelperDesc {
            label: "post.fx.bloom.upsample",
            compute: &cs,
            entry: "cs_upsample",
        },
    )
    .expect("bloom upsample pipeline build")
}

pub(crate) fn build_tonemap_pipeline(device: &Device) -> ComputePipeline {
    let cs = wgsl_artefact_set(Stage::Compute, "cs_main", TONEMAP_WGSL);
    build_compute_pipeline(
        device,
        &ComputePipelineHelperDesc {
            label: "post.fx.tonemap",
            compute: &cs,
            entry: "cs_main",
        },
    )
    .expect("tonemap pipeline build")
}

// =============================================================================
// Front-end culling (PR 3).
// =============================================================================

/// Front-end frustum + occlusion culling. PR 3 lands the frustum-only
/// path; the occlusion query feedback channel is a Phase 6+ follow-up.
#[derive(Debug)]
pub struct CullPass {
    /// Graph handle for the input render queue.
    pub render_queue: ResourceId,
    /// Graph handle for the output indirect-draw buffer.
    pub indirect_draws: ResourceId,
    pipeline: OnceLock<ComputePipeline>,
}

impl CullPass {
    /// Construct with the resource handles the graph builder produced.
    pub fn new(render_queue: ResourceId, indirect_draws: ResourceId) -> Self {
        Self {
            render_queue,
            indirect_draws,
            pipeline: OnceLock::new(),
        }
    }
}

impl Pass for CullPass {
    fn name(&self) -> &'static str {
        "cull"
    }
    fn track(&self) -> Track {
        Track::A
    }
    fn reads(&self, set: &mut ResourceSet) {
        set.add(self.render_queue);
    }
    fn writes(&self, set: &mut ResourceSet) {
        set.add(self.indirect_draws);
    }
    fn record(&mut self, ctx: &mut PassContext) {
        let gpu = ctx
            .gpu
            .as_mut()
            .expect("CullPass.record requires a GpuFrameContext");
        let pipeline = self
            .pipeline
            .get_or_init(|| build_cull_pipeline(gpu.device))
            .clone();
        let mut cpass = gpu.encoder.begin_compute_pass("cull");
        cpass.set_pipeline(&pipeline);
        // PR 7: placeholder dispatch — PR 8 wires the instance count
        // from the RenderQueue resource and divides by
        // `contracts::CULL_WORKGROUP_SIZE[0]`.
        cpass.dispatch_workgroups(1, 1, 1);
    }
}

// =============================================================================
// Cascaded shadow maps (PR 3).
// =============================================================================

/// 4-cascade CSM (ADR-040). One dispatch per cascade; each renders the
/// `ShadowCasters` queue into its quadrant of the 4096² atlas.
#[derive(Debug)]
pub struct CsmShadowPass {
    /// Per-shadow-caster instance queue.
    pub shadow_casters: ResourceId,
    /// 4096² D32F shadow atlas.
    pub shadow_atlas: ResourceId,
    pipeline: OnceLock<RenderPipeline>,
}

impl CsmShadowPass {
    /// Construct with the resource handles the graph builder produced.
    pub fn new(shadow_casters: ResourceId, shadow_atlas: ResourceId) -> Self {
        Self {
            shadow_casters,
            shadow_atlas,
            pipeline: OnceLock::new(),
        }
    }
}

impl Pass for CsmShadowPass {
    fn name(&self) -> &'static str {
        "shadow"
    }
    fn track(&self) -> Track {
        Track::A
    }
    fn reads(&self, set: &mut ResourceSet) {
        set.add(self.shadow_casters);
    }
    fn writes(&self, set: &mut ResourceSet) {
        set.add(self.shadow_atlas);
    }
    fn record(&mut self, ctx: &mut PassContext) {
        let gpu = ctx
            .gpu
            .as_mut()
            .expect("CsmShadowPass.record requires a GpuFrameContext");
        // Lazy-init the depth-only pipeline. PR 8 wires
        // `begin_render_pass` against each cascade's atlas quadrant
        // (4 sub-passes per ADR-040 §3 reverse-Z) plus shadow-caster
        // draws.
        let _ = self
            .pipeline
            .get_or_init(|| build_csm_shadow_pipeline(gpu.device));
    }
}

// =============================================================================
// Cluster-light assignment (PR 3).
// =============================================================================

/// Compute-shader cluster-light assignment. 144 workgroups, 64 threads
/// each (ADR-043 §4); each workgroup walks the 24-slice depth column.
#[derive(Debug)]
pub struct ClusterLightPass {
    /// Per-light SSBO (input).
    pub lights: ResourceId,
    /// Cluster-cell SSBO (output).
    pub cluster_cells: ResourceId,
    pipeline: OnceLock<ComputePipeline>,
}

impl ClusterLightPass {
    /// Construct with the resource handles the graph builder produced.
    pub fn new(lights: ResourceId, cluster_cells: ResourceId) -> Self {
        Self {
            lights,
            cluster_cells,
            pipeline: OnceLock::new(),
        }
    }
}

impl Pass for ClusterLightPass {
    fn name(&self) -> &'static str {
        "light.cluster"
    }
    fn track(&self) -> Track {
        Track::A
    }
    fn reads(&self, set: &mut ResourceSet) {
        set.add(self.lights);
    }
    fn writes(&self, set: &mut ResourceSet) {
        set.add(self.cluster_cells);
    }
    fn record(&mut self, ctx: &mut PassContext) {
        let gpu = ctx
            .gpu
            .as_mut()
            .expect("ClusterLightPass.record requires a GpuFrameContext");
        let pipeline = self
            .pipeline
            .get_or_init(|| build_cluster_assign_pipeline(gpu.device))
            .clone();
        let mut cpass = gpu.encoder.begin_compute_pass("light.cluster");
        cpass.set_pipeline(&pipeline);
        // PR 7: placeholder dispatch — workgroup size is
        // `contracts::CLUSTER_ASSIGN_WORKGROUP_SIZE`. PR 8 supplies
        // the cluster-grid dispatch counts from the
        // `ClusterUniforms.grid_dim` setup.
        cpass.dispatch_workgroups(1, 1, 1);
    }
}

// =============================================================================
// Deferred G-buffer fill (PR 3).
// =============================================================================

/// Deferred MRT G-buffer pass (`draw.opaque`). Writes
/// albedo+roughness, normal+metallic, motion+depth, plus the hardware
/// depth attachment.
#[derive(Debug)]
pub struct GBufferPass {
    /// Cull-pass output.
    pub indirect_draws: ResourceId,
    /// G-buffer attachment: albedo (RGB) + roughness (A).
    pub gbuffer_albedo_roughness: ResourceId,
    /// G-buffer attachment: normal (RG) + metallic (B) + AO (A).
    pub gbuffer_normal_metallic: ResourceId,
    /// G-buffer attachment: motion (RG) + view-z (B).
    pub gbuffer_motion_depth: ResourceId,
    /// Hardware D32F depth (reverse-Z).
    pub depth: ResourceId,
    pipeline: OnceLock<RenderPipeline>,
}

impl GBufferPass {
    /// Construct with the resource handles the graph builder produced.
    pub fn new(
        indirect_draws: ResourceId,
        gbuffer_albedo_roughness: ResourceId,
        gbuffer_normal_metallic: ResourceId,
        gbuffer_motion_depth: ResourceId,
        depth: ResourceId,
    ) -> Self {
        Self {
            indirect_draws,
            gbuffer_albedo_roughness,
            gbuffer_normal_metallic,
            gbuffer_motion_depth,
            depth,
            pipeline: OnceLock::new(),
        }
    }
}

impl Pass for GBufferPass {
    fn name(&self) -> &'static str {
        "draw.opaque"
    }
    fn track(&self) -> Track {
        Track::A
    }
    fn reads(&self, set: &mut ResourceSet) {
        set.add(self.indirect_draws);
    }
    fn writes(&self, set: &mut ResourceSet) {
        set.add(self.gbuffer_albedo_roughness);
        set.add(self.gbuffer_normal_metallic);
        set.add(self.gbuffer_motion_depth);
        set.add(self.depth);
    }
    fn record(&mut self, ctx: &mut PassContext) {
        let gpu = ctx
            .gpu
            .as_mut()
            .expect("GBufferPass.record requires a GpuFrameContext");
        // Lazy-init the 3-MRT + depth pipeline. PR 8 wires the
        // MRT `begin_render_pass` + `draw_indexed_indirect` against
        // the cull-pass-produced `IndirectDrawBuffer`.
        let _ = self
            .pipeline
            .get_or_init(|| build_gbuffer_pipeline(gpu.device));
    }
}

// =============================================================================
// Lighting accumulation (PR 3).
// =============================================================================

/// Lighting accumulation (`draw.opaque.2`). Reads the G-buffer +
/// cluster + light SSBO + shadow atlas; runs Cook-Torrance per light
/// per pixel; writes to `LitColor`.
#[derive(Debug)]
pub struct LightingAccumulationPass {
    /// G-buffer albedo+roughness attachment.
    pub gbuffer_albedo_roughness: ResourceId,
    /// G-buffer normal+metallic attachment.
    pub gbuffer_normal_metallic: ResourceId,
    /// G-buffer motion+view-z attachment.
    pub gbuffer_motion_depth: ResourceId,
    /// Hardware depth (read-only).
    pub depth: ResourceId,
    /// Cluster grid (ADR-043).
    pub cluster_cells: ResourceId,
    /// Per-light SSBO (ADR-043 §3).
    pub lights: ResourceId,
    /// Shadow atlas (ADR-040).
    pub shadow_atlas: ResourceId,
    /// HDR linear-space output.
    pub lit_color: ResourceId,
    pipeline: OnceLock<RenderPipeline>,
}

impl LightingAccumulationPass {
    /// Construct with the resource handles the graph builder produced.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        gbuffer_albedo_roughness: ResourceId,
        gbuffer_normal_metallic: ResourceId,
        gbuffer_motion_depth: ResourceId,
        depth: ResourceId,
        cluster_cells: ResourceId,
        lights: ResourceId,
        shadow_atlas: ResourceId,
        lit_color: ResourceId,
    ) -> Self {
        Self {
            gbuffer_albedo_roughness,
            gbuffer_normal_metallic,
            gbuffer_motion_depth,
            depth,
            cluster_cells,
            lights,
            shadow_atlas,
            lit_color,
            pipeline: OnceLock::new(),
        }
    }
}

impl Pass for LightingAccumulationPass {
    fn name(&self) -> &'static str {
        "draw.opaque.2"
    }
    fn track(&self) -> Track {
        Track::A
    }
    fn reads(&self, set: &mut ResourceSet) {
        set.add(self.gbuffer_albedo_roughness);
        set.add(self.gbuffer_normal_metallic);
        set.add(self.gbuffer_motion_depth);
        set.add(self.depth);
        set.add(self.cluster_cells);
        set.add(self.lights);
        set.add(self.shadow_atlas);
    }
    fn writes(&self, set: &mut ResourceSet) {
        set.add(self.lit_color);
    }
    fn record(&mut self, ctx: &mut PassContext) {
        let gpu = ctx
            .gpu
            .as_mut()
            .expect("LightingAccumulationPass.record requires a GpuFrameContext");
        // Lazy-init the full-screen lighting pipeline (3-vertex
        // triangle, no vertex buffers — full-screen via
        // `@builtin(vertex_index)`). PR 8 wires `begin_render_pass`
        // against the `LitColor` attachment + the Cook-Torrance
        // bind-group reading G-buffer + cluster + shadow.
        let _ = self
            .pipeline
            .get_or_init(|| build_lighting_pipeline(gpu.device));
    }
}

// =============================================================================
// SSAO (PR 4).
// =============================================================================

/// Screen-space ambient-occlusion pass (PR 4). Reads view-space depth +
/// G-buffer normals; writes a single-channel occlusion factor.
#[derive(Debug)]
pub struct SsaoPass {
    /// View-space depth (read from the G-buffer or the hardware
    /// attachment).
    pub depth: ResourceId,
    /// G-buffer normals (RG channels carry the octahedral normal).
    pub gbuffer_normal_metallic: ResourceId,
    /// Single-channel occlusion output.
    pub ssao_target: ResourceId,
    pipeline: OnceLock<ComputePipeline>,
}

impl SsaoPass {
    /// Construct with the resource handles the graph builder produced.
    pub fn new(
        depth: ResourceId,
        gbuffer_normal_metallic: ResourceId,
        ssao_target: ResourceId,
    ) -> Self {
        Self {
            depth,
            gbuffer_normal_metallic,
            ssao_target,
            pipeline: OnceLock::new(),
        }
    }
}

impl Pass for SsaoPass {
    fn name(&self) -> &'static str {
        "post.fx.ssao"
    }
    fn track(&self) -> Track {
        Track::A
    }
    fn reads(&self, set: &mut ResourceSet) {
        set.add(self.depth);
        set.add(self.gbuffer_normal_metallic);
    }
    fn writes(&self, set: &mut ResourceSet) {
        set.add(self.ssao_target);
    }
    fn record(&mut self, ctx: &mut PassContext) {
        let gpu = ctx
            .gpu
            .as_mut()
            .expect("SsaoPass.record requires a GpuFrameContext");
        let pipeline = self
            .pipeline
            .get_or_init(|| build_ssao_pipeline(gpu.device))
            .clone();
        let mut cpass = gpu.encoder.begin_compute_pass("post.fx.ssao");
        cpass.set_pipeline(&pipeline);
        // PR 7: placeholder dispatch — workgroup size is (8,8,1);
        // PR 8 supplies (half-res-width / 8, half-res-height / 8, 1)
        // per `contracts::SSAO_RESOLUTION_DIVISOR`.
        cpass.dispatch_workgroups(1, 1, 1);
    }
}

// =============================================================================
// IBL evaluation (PR 4).
// =============================================================================

/// IBL diffuse + specular accumulation (ADR-041). Reads the L2 SH
/// probe set + the BRDF LUT + the G-buffer; writes the HDR colour
/// target with the IBL contribution.
#[derive(Debug)]
pub struct IblPass {
    /// L2 SH probe set buffer.
    pub probes: ResourceId,
    /// 512×512 Karis split-sum BRDF LUT.
    pub brdf_lut: ResourceId,
    /// G-buffer albedo + roughness.
    pub gbuffer_albedo_roughness: ResourceId,
    /// G-buffer normal + metallic.
    pub gbuffer_normal_metallic: ResourceId,
    /// Hardware depth (used to reconstruct world-space position).
    pub depth: ResourceId,
    /// HDR linear-space output (pre-direct-light target).
    pub lit_color: ResourceId,
    pipeline: OnceLock<ComputePipeline>,
}

impl IblPass {
    /// Construct with the resource handles the graph builder produced.
    pub fn new(
        probes: ResourceId,
        brdf_lut: ResourceId,
        gbuffer_albedo_roughness: ResourceId,
        gbuffer_normal_metallic: ResourceId,
        depth: ResourceId,
        lit_color: ResourceId,
    ) -> Self {
        Self {
            probes,
            brdf_lut,
            gbuffer_albedo_roughness,
            gbuffer_normal_metallic,
            depth,
            lit_color,
            pipeline: OnceLock::new(),
        }
    }
}

impl Pass for IblPass {
    fn name(&self) -> &'static str {
        "draw.opaque.ibl"
    }
    fn track(&self) -> Track {
        Track::A
    }
    fn reads(&self, set: &mut ResourceSet) {
        set.add(self.probes);
        set.add(self.brdf_lut);
        set.add(self.gbuffer_albedo_roughness);
        set.add(self.gbuffer_normal_metallic);
        set.add(self.depth);
    }
    fn writes(&self, set: &mut ResourceSet) {
        set.add(self.lit_color);
    }
    fn record(&mut self, ctx: &mut PassContext) {
        let gpu = ctx
            .gpu
            .as_mut()
            .expect("IblPass.record requires a GpuFrameContext");
        let pipeline = self
            .pipeline
            .get_or_init(|| build_ibl_evaluate_pipeline(gpu.device))
            .clone();
        let mut cpass = gpu.encoder.begin_compute_pass("draw.opaque.ibl");
        cpass.set_pipeline(&pipeline);
        // PR 7: placeholder dispatch — PR 8 supplies the screen
        // tile count derived from the output target size.
        cpass.dispatch_workgroups(1, 1, 1);
    }
}

// =============================================================================
// TAA resolve (PR 4).
// =============================================================================

/// TAA accumulation + history (ADR-042).
#[derive(Debug)]
pub struct TaaPass {
    /// Current-frame HDR colour (lighting accumulation output).
    pub lit_color: ResourceId,
    /// Previous-frame TAA history.
    pub history: ResourceId,
    /// Motion + view-z attachment from the G-buffer pass.
    pub gbuffer_motion_depth: ResourceId,
    /// Hardware depth (used by the disocclusion mask).
    pub depth: ResourceId,
    /// TAA-resolved HDR target (also the canonical upscaler input).
    pub resolved: ResourceId,
    /// Next-frame history slot the pool ping-pongs into.
    pub history_next: ResourceId,
    pipeline: OnceLock<ComputePipeline>,
}

impl TaaPass {
    /// Construct with the resource handles the graph builder produced.
    pub fn new(
        lit_color: ResourceId,
        history: ResourceId,
        gbuffer_motion_depth: ResourceId,
        depth: ResourceId,
        resolved: ResourceId,
        history_next: ResourceId,
    ) -> Self {
        Self {
            lit_color,
            history,
            gbuffer_motion_depth,
            depth,
            resolved,
            history_next,
            pipeline: OnceLock::new(),
        }
    }
}

impl Pass for TaaPass {
    fn name(&self) -> &'static str {
        "post.fx.taa"
    }
    fn track(&self) -> Track {
        Track::A
    }
    fn reads(&self, set: &mut ResourceSet) {
        set.add(self.lit_color);
        set.add(self.history);
        set.add(self.gbuffer_motion_depth);
        set.add(self.depth);
    }
    fn writes(&self, set: &mut ResourceSet) {
        set.add(self.resolved);
        set.add(self.history_next);
    }
    fn record(&mut self, ctx: &mut PassContext) {
        let gpu = ctx
            .gpu
            .as_mut()
            .expect("TaaPass.record requires a GpuFrameContext");
        let pipeline = self
            .pipeline
            .get_or_init(|| build_taa_resolve_pipeline(gpu.device))
            .clone();
        let mut cpass = gpu.encoder.begin_compute_pass("post.fx.taa");
        cpass.set_pipeline(&pipeline);
        // PR 7: placeholder dispatch. PR 8 supplies the screen-tile
        // count + the jitter uniform via
        // `engine_raster::post_fx::jitter_for_frame(ctx.frame_idx)`.
        cpass.dispatch_workgroups(1, 1, 1);
    }
}

// =============================================================================
// Bloom (PR 4) — three compute pipelines: extract, downsample, upsample.
// =============================================================================

/// Bloom extract + blur (PR 4). Reads the TAA-resolved HDR target;
/// writes the low-frequency bright-pass layer for the tonemap pass to
/// composite.
#[derive(Debug)]
pub struct BloomPass {
    /// TAA-resolved HDR input.
    pub resolved: ResourceId,
    /// Bloom layer output.
    pub bloom_target: ResourceId,
    pipeline_extract: OnceLock<ComputePipeline>,
    pipeline_downsample: OnceLock<ComputePipeline>,
    pipeline_upsample: OnceLock<ComputePipeline>,
}

impl BloomPass {
    /// Construct with the resource handles the graph builder produced.
    pub fn new(resolved: ResourceId, bloom_target: ResourceId) -> Self {
        Self {
            resolved,
            bloom_target,
            pipeline_extract: OnceLock::new(),
            pipeline_downsample: OnceLock::new(),
            pipeline_upsample: OnceLock::new(),
        }
    }
}

impl Pass for BloomPass {
    fn name(&self) -> &'static str {
        "post.fx.bloom"
    }
    fn track(&self) -> Track {
        Track::A
    }
    fn reads(&self, set: &mut ResourceSet) {
        set.add(self.resolved);
    }
    fn writes(&self, set: &mut ResourceSet) {
        set.add(self.bloom_target);
    }
    fn record(&mut self, ctx: &mut PassContext) {
        let gpu = ctx
            .gpu
            .as_mut()
            .expect("BloomPass.record requires a GpuFrameContext");
        // Lazy-init the three bloom pipelines. PR 8 wires the
        // down-sample chain + up-sample composite per ADR-065 §5
        // (5 mip levels = `contracts::BLOOM_MIP_LEVELS`).
        let extract = self
            .pipeline_extract
            .get_or_init(|| build_bloom_extract_pipeline(gpu.device))
            .clone();
        let _down = self
            .pipeline_downsample
            .get_or_init(|| build_bloom_downsample_pipeline(gpu.device));
        let _up = self
            .pipeline_upsample
            .get_or_init(|| build_bloom_upsample_pipeline(gpu.device));
        let mut cpass = gpu.encoder.begin_compute_pass("post.fx.bloom.extract");
        cpass.set_pipeline(&extract);
        // PR 7: placeholder extract dispatch. PR 8 chains
        // downsample + upsample dispatches across the mip chain.
        cpass.dispatch_workgroups(1, 1, 1);
    }
}

// =============================================================================
// Upscale (PR 5) — dispatches through the UpscalerRegistry.
// =============================================================================

/// Upscale pass (PR 5, ADR-005 + ADR-053). The pass body adapts the
/// active [`crate::upscale::UpscalerProvider`] into the render-graph
/// schedule; vendor SDK dispatch lands in PR 8.
///
/// Skipping the upscale pass (no-upscale variant) is the PR-4 graph
/// shape: bloom + tonemap read `TaaResolvedColor` directly. With the
/// upscale variant, bloom still extracts from the TAA-resolved buffer
/// (chroma + energy invariants) and tonemap reads `upscaled` for its
/// HDR input.
#[derive(Debug, Clone, Copy)]
pub struct UpscalePass {
    /// TAA-resolved HDR input (internal resolution).
    pub resolved: ResourceId,
    /// Upscaled HDR output (display resolution).
    pub upscaled: ResourceId,
}

impl UpscalePass {
    /// Construct with the resource handles the graph builder produced.
    pub fn new(resolved: ResourceId, upscaled: ResourceId) -> Self {
        Self { resolved, upscaled }
    }
}

impl Pass for UpscalePass {
    fn name(&self) -> &'static str {
        "post.fx.upscale"
    }
    fn track(&self) -> Track {
        Track::A
    }
    fn reads(&self, set: &mut ResourceSet) {
        set.add(self.resolved);
    }
    fn writes(&self, set: &mut ResourceSet) {
        set.add(self.upscaled);
    }
    fn record(&mut self, _ctx: &mut PassContext) {
        // PR 7: no-op. The upscaler dispatches through
        // [`crate::upscale::UpscalerRegistry`] which is not yet
        // threaded through `PassContext`; PR 8 wires the registry
        // lookup + `provider.upscale(&mut UpscaleCtx { .. })` call.
        // CPU oracle reference: `engine_raster::upscale::bilinear_upscale`.
    }
}

// =============================================================================
// Tonemap (PR 4).
// =============================================================================

/// Tonemap + bloom composite (PR 4). Reads the TAA-resolved HDR + the
/// bloom layer; writes the final LDR target (`TonemappedColor`).
#[derive(Debug)]
pub struct TonemapPass {
    /// TAA-resolved HDR input.
    pub resolved: ResourceId,
    /// Bloom layer.
    pub bloom: ResourceId,
    /// LDR output.
    pub tonemapped: ResourceId,
    pipeline: OnceLock<ComputePipeline>,
}

impl TonemapPass {
    /// Construct with the resource handles the graph builder produced.
    pub fn new(resolved: ResourceId, bloom: ResourceId, tonemapped: ResourceId) -> Self {
        Self {
            resolved,
            bloom,
            tonemapped,
            pipeline: OnceLock::new(),
        }
    }
}

impl Pass for TonemapPass {
    fn name(&self) -> &'static str {
        "post.fx.tonemap"
    }
    fn track(&self) -> Track {
        Track::A
    }
    fn reads(&self, set: &mut ResourceSet) {
        set.add(self.resolved);
        set.add(self.bloom);
    }
    fn writes(&self, set: &mut ResourceSet) {
        set.add(self.tonemapped);
    }
    fn record(&mut self, ctx: &mut PassContext) {
        let gpu = ctx
            .gpu
            .as_mut()
            .expect("TonemapPass.record requires a GpuFrameContext");
        let pipeline = self
            .pipeline
            .get_or_init(|| build_tonemap_pipeline(gpu.device))
            .clone();
        let mut cpass = gpu.encoder.begin_compute_pass("post.fx.tonemap");
        cpass.set_pipeline(&pipeline);
        // PR 7: placeholder dispatch. PR 8 wires the LDR output tile
        // count + the ACES exposure / bloom-mix uniforms.
        cpass.dispatch_workgroups(1, 1, 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render_graph::RenderGraph;

    /// Smoke test: registering the five PR-3 passes in canonical order
    /// produces a green compile and the expected scheduling.
    #[test]
    fn pr3_passes_schedule_in_canonical_order() {
        let mut g = RenderGraph::new();
        let queue = ResourceId(0);
        let casters = ResourceId(1);
        let lights = ResourceId(2);
        let indirect = ResourceId(3);
        let shadow_atlas = ResourceId(4);
        let cluster_cells = ResourceId(5);
        let gbuf_ar = ResourceId(6);
        let gbuf_nm = ResourceId(7);
        let gbuf_md = ResourceId(8);
        let depth = ResourceId(9);
        let lit = ResourceId(10);

        g.add_pass(CullPass::new(queue, indirect));
        g.add_pass(CsmShadowPass::new(casters, shadow_atlas));
        g.add_pass(ClusterLightPass::new(lights, cluster_cells));
        g.add_pass(GBufferPass::new(indirect, gbuf_ar, gbuf_nm, gbuf_md, depth));
        g.add_pass(LightingAccumulationPass::new(
            gbuf_ar,
            gbuf_nm,
            gbuf_md,
            depth,
            cluster_cells,
            lights,
            shadow_atlas,
            lit,
        ));
        let n = g.compile().expect("graph compiles");
        assert_eq!(n, 5);
        let names = g.scheduled_names().unwrap();
        assert_eq!(names[0], "cull");
        let cull_idx = names.iter().position(|&n| n == "cull").unwrap();
        let draw_idx = names.iter().position(|&n| n == "draw.opaque").unwrap();
        let lighting_idx = names.iter().position(|&n| n == "draw.opaque.2").unwrap();
        let shadow_idx = names.iter().position(|&n| n == "shadow").unwrap();
        let cluster_idx = names.iter().position(|&n| n == "light.cluster").unwrap();
        assert!(cull_idx < draw_idx, "cull before draw.opaque");
        assert!(draw_idx < lighting_idx, "draw.opaque before draw.opaque.2");
        assert!(shadow_idx < lighting_idx, "shadow before lighting");
        assert!(cluster_idx < lighting_idx, "cluster before lighting");
    }

    /// PR-4 smoke test: SSAO + IBL + TAA + Bloom + Tonemap slot in
    /// after the PR-3 G-buffer + lighting chain in the canonical order.
    #[test]
    fn pr4_post_fx_chain_schedules_after_lighting() {
        let mut g = RenderGraph::new();
        let queue = ResourceId(0);
        let casters = ResourceId(1);
        let lights = ResourceId(2);
        let indirect = ResourceId(3);
        let shadow_atlas = ResourceId(4);
        let cluster_cells = ResourceId(5);
        let gbuf_ar = ResourceId(6);
        let gbuf_nm = ResourceId(7);
        let gbuf_md = ResourceId(8);
        let depth = ResourceId(9);
        let lit = ResourceId(10);
        let probes = ResourceId(11);
        let brdf_lut = ResourceId(12);
        let ssao = ResourceId(13);
        let taa_history_prev = ResourceId(14);
        let taa_history_next = ResourceId(15);
        let taa_resolved = ResourceId(16);
        let bloom = ResourceId(17);
        let tonemapped = ResourceId(18);

        g.add_pass(CullPass::new(queue, indirect));
        g.add_pass(CsmShadowPass::new(casters, shadow_atlas));
        g.add_pass(ClusterLightPass::new(lights, cluster_cells));
        g.add_pass(GBufferPass::new(indirect, gbuf_ar, gbuf_nm, gbuf_md, depth));
        g.add_pass(SsaoPass::new(depth, gbuf_nm, ssao));
        g.add_pass(IblPass::new(probes, brdf_lut, gbuf_ar, gbuf_nm, depth, lit));
        g.add_pass(LightingAccumulationPass::new(
            gbuf_ar,
            gbuf_nm,
            gbuf_md,
            depth,
            cluster_cells,
            lights,
            shadow_atlas,
            lit,
        ));
        g.add_pass(TaaPass::new(
            lit,
            taa_history_prev,
            gbuf_md,
            depth,
            taa_resolved,
            taa_history_next,
        ));
        g.add_pass(BloomPass::new(taa_resolved, bloom));
        g.add_pass(TonemapPass::new(taa_resolved, bloom, tonemapped));
        let n = g.compile().expect("graph compiles");
        assert_eq!(n, 10);
        let names = g.scheduled_names().unwrap();
        let pos = |needle: &str| names.iter().position(|&s| s == needle).unwrap();
        let gbuf_idx = pos("draw.opaque");
        let ssao_idx = pos("post.fx.ssao");
        let ibl_idx = pos("draw.opaque.ibl");
        let lighting_idx = pos("draw.opaque.2");
        let taa_idx = pos("post.fx.taa");
        let bloom_idx = pos("post.fx.bloom");
        let tonemap_idx = pos("post.fx.tonemap");
        assert!(gbuf_idx < ssao_idx, "g-buffer before ssao");
        assert!(gbuf_idx < ibl_idx, "g-buffer before ibl");
        assert!(lighting_idx < taa_idx, "lighting before taa");
        assert!(ibl_idx < taa_idx, "ibl before taa");
        assert!(taa_idx < bloom_idx, "taa before bloom");
        assert!(taa_idx < tonemap_idx, "taa before tonemap");
        assert!(bloom_idx < tonemap_idx, "bloom before tonemap");
    }

    /// PR-5 smoke test: the upscale-path variant schedules
    /// `taa → upscale → tonemap` with bloom still feeding off the
    /// TAA-resolved buffer.
    #[test]
    fn pr5_upscale_variant_schedules_taa_upscale_tonemap() {
        let mut g = RenderGraph::new();
        let queue = ResourceId(0);
        let casters = ResourceId(1);
        let lights = ResourceId(2);
        let indirect = ResourceId(3);
        let shadow_atlas = ResourceId(4);
        let cluster_cells = ResourceId(5);
        let gbuf_ar = ResourceId(6);
        let gbuf_nm = ResourceId(7);
        let gbuf_md = ResourceId(8);
        let depth = ResourceId(9);
        let lit = ResourceId(10);
        let taa_history_prev = ResourceId(11);
        let taa_history_next = ResourceId(12);
        let taa_resolved = ResourceId(13);
        let upscaled = ResourceId(14);
        let bloom = ResourceId(15);
        let tonemapped = ResourceId(16);

        g.add_pass(CullPass::new(queue, indirect));
        g.add_pass(CsmShadowPass::new(casters, shadow_atlas));
        g.add_pass(ClusterLightPass::new(lights, cluster_cells));
        g.add_pass(GBufferPass::new(indirect, gbuf_ar, gbuf_nm, gbuf_md, depth));
        g.add_pass(LightingAccumulationPass::new(
            gbuf_ar,
            gbuf_nm,
            gbuf_md,
            depth,
            cluster_cells,
            lights,
            shadow_atlas,
            lit,
        ));
        g.add_pass(TaaPass::new(
            lit,
            taa_history_prev,
            gbuf_md,
            depth,
            taa_resolved,
            taa_history_next,
        ));
        g.add_pass(UpscalePass::new(taa_resolved, upscaled));
        g.add_pass(BloomPass::new(taa_resolved, bloom));
        g.add_pass(TonemapPass::new(upscaled, bloom, tonemapped));

        let n = g.compile().expect("graph compiles");
        assert_eq!(n, 9);
        let names = g.scheduled_names().unwrap();
        let pos = |needle: &str| names.iter().position(|&s| s == needle).unwrap();
        let taa_idx = pos("post.fx.taa");
        let upscale_idx = pos("post.fx.upscale");
        let bloom_idx = pos("post.fx.bloom");
        let tonemap_idx = pos("post.fx.tonemap");
        assert!(taa_idx < upscale_idx, "taa before upscale");
        assert!(taa_idx < bloom_idx, "taa before bloom");
        assert!(upscale_idx < tonemap_idx, "upscale before tonemap");
        assert!(bloom_idx < tonemap_idx, "bloom before tonemap");
    }
}
