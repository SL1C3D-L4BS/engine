//! Phase 5 PR 3 + PR 4 + PR 7 + PR 7.5 — ten deferred render-graph passes.
//!
//! All implementations name [`engine_gpu`] types; none names `wgpu`
//! directly (ADR-049). Each pass owns a GPU pipeline built once via
//! [`Pass::install_pipeline`] (called by
//! [`crate::render_graph::RenderGraph::install_pipelines`] at renderer
//! startup); the `record()` body binds it. PR 7 introduced the
//! construction surface; PR 7.5 lifted the per-frame `.expect()`
//! panic risk by moving the build to eager startup.
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
//! ## PR 7.5 wiring scope
//!
//! Each pass owns a `pipeline: Option<{Render,Compute}Pipeline>` slot
//! that [`Pass::install_pipeline`] fills at startup from the embedded
//! WGSL constants in [`crate::shaders`] via
//! [`crate::shader::wgsl_artefact_set`]. The `record()` body
//! short-circuits if either (a) no GPU command surface is bound
//! (`ctx.gpu == None` — the CPU-rasterizer testbed path) or (b) no
//! pipeline has been installed (graphs built for scheduling tests).
//! Compute-pass scopes (begin → set_pipeline → dispatch) issue
//! placeholder dispatch dimensions; PR 8 wires real resource lookups +
//! bind groups + dispatch counts. Render passes (depth-only, MRT
//! G-buffer, full-screen lighting) defer `begin_render_pass` to PR 8
//! because the current [`engine_gpu`] surface requires attachment
//! [`engine_gpu::TextureView`]s that the render graph's transient
//! resource pool doesn't yet resolve.

use engine_gpu::{
    BindGroup, BindGroupDesc, BindGroupEntry, BindingResource, Color, ColorTargetState,
    ComputePipeline, DepthLoadOp, DepthStencilState, Device, LoadOp, RenderPassColorAttachment,
    RenderPassDepthAttachment, RenderPassDesc, RenderPipeline, VertexAttribute, VertexBufferLayout,
    VertexFormat, VertexStepMode,
};
use engine_shader::Stage;

/// Byte size of the `InstanceEntry` SSBO record consumed by the cull
/// shader (`shaders/cull.wgsl`). Mirrors the WGSL `struct InstanceEntry`
/// layout: 32 B AABB + 4 × u32 = 48 bytes. CullPass derives its
/// dispatch count from `instances_buffer.size() / INSTANCE_ENTRY_SIZE`.
/// Keep in sync with the shader; a discrepancy is silently a wrong
/// dispatch count (over-dispatch is OK; under-dispatch silently culls
/// instances).
const INSTANCE_ENTRY_SIZE: u64 = 48;

/// Helper: extract `(width, height)` from a [`engine_gpu::TextureView`]
/// for dispatch-count derivation in screen-space compute passes.
/// The `TextureView`'s extent comes from the underlying [`Texture`]
/// (Phase 5.5 A.2b-ii widened `TextureView` to carry the extent +
/// format). Passes use this to compute `dim.div_ceil(workgroup_size)`.
fn dispatch_dim_for_view(view: &engine_gpu::TextureView<'_>) -> (u32, u32) {
    let e = view.extent();
    (e.width, e.height)
}

/// Interleaved mesh-vertex layout (position + normal + tangent + uv0) —
/// the EMSH binary format's `MeshVertex` struct. ADR-061 §1 pins the
/// layout: 12 + 12 + 16 + 8 = 48 bytes per vertex. Both `GBufferPass`
/// and `CsmShadowPass` consume this layout; the shadow path declares
/// only `@location(0)` (position), but the same buffer feeds both
/// pipelines because wgpu only validates declared locations.
const MESH_VERTEX_ATTRIBUTES: &[VertexAttribute] = &[
    VertexAttribute {
        offset: 0,
        shader_location: 0,
        format: VertexFormat::Float32x3,
    },
    VertexAttribute {
        offset: 12,
        shader_location: 1,
        format: VertexFormat::Float32x3,
    },
    VertexAttribute {
        offset: 24,
        shader_location: 2,
        format: VertexFormat::Float32x4,
    },
    VertexAttribute {
        offset: 40,
        shader_location: 3,
        format: VertexFormat::Float32x2,
    },
];

fn mesh_vertex_buffer_layout() -> [VertexBufferLayout<'static>; 1] {
    [VertexBufferLayout {
        array_stride: 48,
        step_mode: VertexStepMode::Vertex,
        attributes: MESH_VERTEX_ATTRIBUTES,
    }]
}

use crate::contracts;
use crate::contracts::{
    DEPTH_BUFFER_FORMAT, GBUFFER_ALBEDO_ROUGHNESS_FORMAT, GBUFFER_MOTION_DEPTH_FORMAT,
    GBUFFER_NORMAL_METALLIC_FORMAT, LIT_COLOR_FORMAT,
};
use crate::render_graph::{Pass, PassContext, ResourceId, ResourceSet, Track};
use crate::shader::{
    ComputePipelineHelperDesc, RenderPipelineHelperDesc, ShaderError, build_compute_pipeline,
    build_render_pipeline, wgsl_artefact_set,
};
use crate::shaders::{
    BLOOM_WGSL, CLUSTER_ASSIGN_WGSL, CSM_SHADOW_WGSL, CULL_WGSL, GBUFFER_WGSL, IBL_EVALUATE_WGSL,
    LIGHTING_WGSL, SSAO_WGSL, TAA_RESOLVE_WGSL, TONEMAP_WGSL,
};

// =============================================================================
// Pipeline-builder helpers (one per pass / per entry point).
// =============================================================================

pub(crate) fn build_cull_pipeline(device: &Device) -> Result<ComputePipeline, ShaderError> {
    let cs = wgsl_artefact_set(Stage::Compute, "cs_main", CULL_WGSL);
    build_compute_pipeline(
        device,
        &ComputePipelineHelperDesc {
            label: "cull",
            compute: &cs,
            entry: "cs_main",
        },
    )
}

pub(crate) fn build_csm_shadow_pipeline(device: &Device) -> Result<RenderPipeline, ShaderError> {
    let vs = wgsl_artefact_set(Stage::Vertex, "vs_main", CSM_SHADOW_WGSL);
    let buffers = mesh_vertex_buffer_layout();
    build_render_pipeline(
        device,
        &RenderPipelineHelperDesc {
            label: "shadow",
            vertex: &vs,
            vertex_entry: "vs_main",
            vertex_buffers: &buffers,
            fragment: None,
            fragment_entry: "",
            color_targets: &[],
            depth_stencil: Some(DepthStencilState {
                format: DEPTH_BUFFER_FORMAT,
                depth_write_enabled: true,
            }),
        },
    )
}

pub(crate) fn build_cluster_assign_pipeline(
    device: &Device,
) -> Result<ComputePipeline, ShaderError> {
    let cs = wgsl_artefact_set(Stage::Compute, "cs_main", CLUSTER_ASSIGN_WGSL);
    build_compute_pipeline(
        device,
        &ComputePipelineHelperDesc {
            label: "light.cluster",
            compute: &cs,
            entry: "cs_main",
        },
    )
}

pub(crate) fn build_gbuffer_pipeline(device: &Device) -> Result<RenderPipeline, ShaderError> {
    let vs = wgsl_artefact_set(Stage::Vertex, "vs_main", GBUFFER_WGSL);
    let fs = wgsl_artefact_set(Stage::Fragment, "fs_main", GBUFFER_WGSL);
    let buffers = mesh_vertex_buffer_layout();
    build_render_pipeline(
        device,
        &RenderPipelineHelperDesc {
            // Label matches `GBufferPass::name()` so trace-correlation
            // tooling joining on schedule names against encoder labels
            // sees identical strings.
            label: "draw.opaque",
            vertex: &vs,
            vertex_entry: "vs_main",
            vertex_buffers: &buffers,
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
}

pub(crate) fn build_ssao_pipeline(device: &Device) -> Result<ComputePipeline, ShaderError> {
    let cs = wgsl_artefact_set(Stage::Compute, "cs_main", SSAO_WGSL);
    build_compute_pipeline(
        device,
        &ComputePipelineHelperDesc {
            label: "post.fx.ssao",
            compute: &cs,
            entry: "cs_main",
        },
    )
}

pub(crate) fn build_ibl_evaluate_pipeline(device: &Device) -> Result<ComputePipeline, ShaderError> {
    let cs = wgsl_artefact_set(Stage::Compute, "cs_main", IBL_EVALUATE_WGSL);
    build_compute_pipeline(
        device,
        &ComputePipelineHelperDesc {
            label: "draw.opaque.ibl",
            compute: &cs,
            entry: "cs_main",
        },
    )
}

pub(crate) fn build_lighting_pipeline(device: &Device) -> Result<RenderPipeline, ShaderError> {
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
}

pub(crate) fn build_taa_resolve_pipeline(device: &Device) -> Result<ComputePipeline, ShaderError> {
    let cs = wgsl_artefact_set(Stage::Compute, "cs_main", TAA_RESOLVE_WGSL);
    build_compute_pipeline(
        device,
        &ComputePipelineHelperDesc {
            label: "post.fx.taa",
            compute: &cs,
            entry: "cs_main",
        },
    )
}

pub(crate) fn build_bloom_extract_pipeline(
    device: &Device,
) -> Result<ComputePipeline, ShaderError> {
    let cs = wgsl_artefact_set(Stage::Compute, "cs_extract", BLOOM_WGSL);
    build_compute_pipeline(
        device,
        &ComputePipelineHelperDesc {
            label: "post.fx.bloom.extract",
            compute: &cs,
            entry: "cs_extract",
        },
    )
}

pub(crate) fn build_bloom_downsample_pipeline(
    device: &Device,
) -> Result<ComputePipeline, ShaderError> {
    let cs = wgsl_artefact_set(Stage::Compute, "cs_downsample", BLOOM_WGSL);
    build_compute_pipeline(
        device,
        &ComputePipelineHelperDesc {
            label: "post.fx.bloom.downsample",
            compute: &cs,
            entry: "cs_downsample",
        },
    )
}

pub(crate) fn build_bloom_upsample_pipeline(
    device: &Device,
) -> Result<ComputePipeline, ShaderError> {
    let cs = wgsl_artefact_set(Stage::Compute, "cs_upsample", BLOOM_WGSL);
    build_compute_pipeline(
        device,
        &ComputePipelineHelperDesc {
            label: "post.fx.bloom.upsample",
            compute: &cs,
            entry: "cs_upsample",
        },
    )
}

pub(crate) fn build_tonemap_pipeline(device: &Device) -> Result<ComputePipeline, ShaderError> {
    let cs = wgsl_artefact_set(Stage::Compute, "cs_main", TONEMAP_WGSL);
    build_compute_pipeline(
        device,
        &ComputePipelineHelperDesc {
            label: "post.fx.tonemap",
            compute: &cs,
            entry: "cs_main",
        },
    )
}

// =============================================================================
// Front-end culling (PR 3).
// =============================================================================

/// Front-end frustum + occlusion culling. PR 3 lands the frustum-only
/// path; the occlusion query feedback channel is a Phase 6+ follow-up.
#[derive(Debug)]
pub struct CullPass {
    /// Graph handle for the per-frame instance SSBO
    /// (`shaders/cull.wgsl` `@group(0) @binding(1)`).
    pub render_queue: ResourceId,
    /// Graph handle for the output indirect-draw SSBO
    /// (`@group(0) @binding(3)`).
    pub indirect_draws: ResourceId,
    /// Graph handle for the per-frame frustum UBO
    /// (`@group(0) @binding(0)`).
    pub frustum_uniforms: ResourceId,
    /// Graph handle for the static mesh table SSBO
    /// (`@group(0) @binding(2)`).
    pub meshes: ResourceId,
    /// Graph handle for the draw-count atomic SSBO
    /// (`@group(0) @binding(4)`).
    pub draw_count: ResourceId,
    pipeline: Option<ComputePipeline>,
}

impl CullPass {
    /// Construct with the resource handles the graph builder produced.
    pub fn new(
        render_queue: ResourceId,
        indirect_draws: ResourceId,
        frustum_uniforms: ResourceId,
        meshes: ResourceId,
        draw_count: ResourceId,
    ) -> Self {
        Self {
            render_queue,
            indirect_draws,
            frustum_uniforms,
            meshes,
            draw_count,
            pipeline: None,
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
    fn install_pipeline(&mut self, device: &Device) -> Result<(), ShaderError> {
        self.pipeline = Some(build_cull_pipeline(device)?);
        Ok(())
    }
    fn record(&mut self, ctx: &mut PassContext) {
        // Step 1: CPU-oracle short-circuit (ADR-075 §1).
        let Some(gpu) = ctx.gpu.as_mut() else {
            return;
        };
        // Step 2: pipeline-installed short-circuit.
        let Some(pipeline) = self.pipeline.as_ref() else {
            return;
        };
        // Step 3: resolver short-circuit (graph used in CPU-only mode).
        let Some(resources) = ctx.resources else {
            return;
        };
        // Step 4: resolve all bindings against `shaders/cull.wgsl`'s
        //         `@group(0)` declarations. Short-circuit on first
        //         missing — the renderer is in mid-build.
        let Some(frustum) = resources.resolve_buffer(self.frustum_uniforms) else {
            return;
        };
        let Some(instances) = resources.resolve_buffer(self.render_queue) else {
            return;
        };
        let Some(meshes) = resources.resolve_buffer(self.meshes) else {
            return;
        };
        let Some(draws) = resources.resolve_buffer(self.indirect_draws) else {
            return;
        };
        let Some(draw_count) = resources.resolve_buffer(self.draw_count) else {
            return;
        };
        let layout = pipeline.bind_group_layout(0);
        let bind_group = BindGroup::new(
            gpu.device,
            &BindGroupDesc {
                label: "cull.bindgroup",
                layout: &layout,
                entries: &[
                    BindGroupEntry {
                        binding: 0,
                        resource: BindingResource::Buffer(frustum),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: BindingResource::Buffer(instances),
                    },
                    BindGroupEntry {
                        binding: 2,
                        resource: BindingResource::Buffer(meshes),
                    },
                    BindGroupEntry {
                        binding: 3,
                        resource: BindingResource::Buffer(draws),
                    },
                    BindGroupEntry {
                        binding: 4,
                        resource: BindingResource::Buffer(draw_count),
                    },
                ],
            },
        );
        // Step 5: open the pass scope, bind, dispatch. The shader's
        //         (64, 1, 1) workgroup processes one instance per
        //         thread; the dispatch count is
        //         ceil(instance_count / CULL_WORKGROUP_SIZE.x).
        //         Instance count derives from the SSBO size — the
        //         shader's `if (idx >= arrayLength(...)) return;`
        //         absorbs over-dispatch from rounding.
        let instance_count = (instances.size() / INSTANCE_ENTRY_SIZE).max(1) as u32;
        let dispatch_x = instance_count.div_ceil(contracts::CULL_WORKGROUP_SIZE[0]);
        let mut cpass = gpu.encoder.begin_compute_pass(self.name());
        cpass.set_pipeline(pipeline);
        cpass.set_bind_group(0, &bind_group);
        cpass.dispatch_workgroups(dispatch_x, 1, 1);
        // Step 6: end-of-scope drops `cpass`; the encoder is owned by
        //         `RenderGraph::execute` and submitted once per frame.
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
    /// 4096² D32F shadow atlas (depth attachment).
    pub shadow_atlas: ResourceId,
    /// CSM uniforms UBO (`@group(1) @binding(0)`).
    pub csm_uniforms: ResourceId,
    pipeline: Option<RenderPipeline>,
}

impl CsmShadowPass {
    /// Construct with the resource handles the graph builder produced.
    pub fn new(
        shadow_casters: ResourceId,
        shadow_atlas: ResourceId,
        csm_uniforms: ResourceId,
    ) -> Self {
        Self {
            shadow_casters,
            shadow_atlas,
            csm_uniforms,
            pipeline: None,
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
    fn install_pipeline(&mut self, device: &Device) -> Result<(), ShaderError> {
        self.pipeline = Some(build_csm_shadow_pipeline(device)?);
        Ok(())
    }
    fn record(&mut self, ctx: &mut PassContext) {
        // ADR-075 §1 — six-step template, depth-only render pass.
        //
        // The pass opens a depth-only render pass against the shadow
        // atlas and clears the depth attachment (reverse-Z convention
        // clears to 0.0 at the far plane). Per-draw push constants +
        // vertex/index/indirect binding for the
        // `ShadowCasters` queue land in A.2d — they require:
        // (a) Per-draw push constants flow (the WGSL `var<immediate>`
        //     declaration), and
        // (b) Indirect-draw consumption pattern (one `draw_indexed_indirect`
        //     per surviving caster after culling, or a WGSL refactor
        //     to read model transforms from a per-instance SSBO).
        //
        // The clear is meaningful work: subsequent passes that sample
        // the shadow atlas without a writer would otherwise see
        // undefined depth. Step 5 lays the pass scope; A.2d fills in
        // the draws.
        let Some(gpu) = ctx.gpu.as_mut() else {
            return;
        };
        let Some(pipeline) = self.pipeline.as_ref() else {
            return;
        };
        let Some(resources) = ctx.resources else {
            return;
        };
        let Some(_csm_u) = resources.resolve_buffer(self.csm_uniforms) else {
            return;
        };
        let Some(atlas) = resources.resolve_view(self.shadow_atlas) else {
            return;
        };
        // Depth-only render pass; reverse-Z clear at 0.0 (far plane).
        let mut rpass = gpu.encoder.begin_render_pass_desc(&RenderPassDesc {
            label: "shadow.renderpass",
            color_attachments: &[],
            depth: Some(RenderPassDepthAttachment {
                view: &atlas,
                load: DepthLoadOp::Clear(0.0),
                store: true,
            }),
        });
        rpass.set_pipeline(pipeline);
        // A.2d: per-cascade viewport + per-draw push constants +
        // `draw_indexed_indirect` against the cull pass's draw-arg
        // buffer. Until then the pass clears the atlas and ends.
    }
}

// =============================================================================
// Cluster-light assignment (PR 3).
// =============================================================================

/// Compute-shader cluster-light assignment. 144 workgroups, 64 threads
/// each (ADR-043 §4); each workgroup walks the 24-slice depth column.
#[derive(Debug)]
pub struct ClusterLightPass {
    /// Per-light SSBO (input, `@group(1) @binding(1)`).
    pub lights: ResourceId,
    /// Cluster-cell SSBO (output, `@group(1) @binding(2)`).
    pub cluster_cells: ResourceId,
    /// Cluster UBO (`@group(1) @binding(0)`).
    pub cluster_uniforms: ResourceId,
    /// Light-indices SSBO (`@group(1) @binding(3)`).
    pub light_indices: ResourceId,
    /// Atomic indices-cursor SSBO (`@group(1) @binding(4)`).
    pub indices_cursor: ResourceId,
    pipeline: Option<ComputePipeline>,
}

impl ClusterLightPass {
    /// Construct with the resource handles the graph builder produced.
    pub fn new(
        lights: ResourceId,
        cluster_cells: ResourceId,
        cluster_uniforms: ResourceId,
        light_indices: ResourceId,
        indices_cursor: ResourceId,
    ) -> Self {
        Self {
            lights,
            cluster_cells,
            cluster_uniforms,
            light_indices,
            indices_cursor,
            pipeline: None,
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
    fn install_pipeline(&mut self, device: &Device) -> Result<(), ShaderError> {
        self.pipeline = Some(build_cluster_assign_pipeline(device)?);
        Ok(())
    }
    fn record(&mut self, ctx: &mut PassContext) {
        // ADR-075 §1 — six-step template.
        let Some(gpu) = ctx.gpu.as_mut() else {
            return;
        };
        let Some(pipeline) = self.pipeline.as_ref() else {
            return;
        };
        let Some(resources) = ctx.resources else {
            return;
        };
        let Some(cluster) = resources.resolve_buffer(self.cluster_uniforms) else {
            return;
        };
        let Some(lights) = resources.resolve_buffer(self.lights) else {
            return;
        };
        let Some(cells) = resources.resolve_buffer(self.cluster_cells) else {
            return;
        };
        let Some(indices) = resources.resolve_buffer(self.light_indices) else {
            return;
        };
        let Some(cursor) = resources.resolve_buffer(self.indices_cursor) else {
            return;
        };
        // Cluster-assign uses `@group(1)`, not `@group(0)` — the
        // auto-derived layout reflects the WGSL declaration. Auto-
        // derive may emit a sparse layout array; querying set 1
        // returns the right one.
        let layout = pipeline.bind_group_layout(1);
        let bind_group = BindGroup::new(
            gpu.device,
            &BindGroupDesc {
                label: "light.cluster.bindgroup",
                layout: &layout,
                entries: &[
                    BindGroupEntry {
                        binding: 0,
                        resource: BindingResource::Buffer(cluster),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: BindingResource::Buffer(lights),
                    },
                    BindGroupEntry {
                        binding: 2,
                        resource: BindingResource::Buffer(cells),
                    },
                    BindGroupEntry {
                        binding: 3,
                        resource: BindingResource::Buffer(indices),
                    },
                    BindGroupEntry {
                        binding: 4,
                        resource: BindingResource::Buffer(cursor),
                    },
                ],
            },
        );
        // Shader workgroup is (16, 9, 1) — matches the X+Y cluster
        // grid dimensions exactly. One workgroup covers the whole
        // XY plane; the inner loop walks Z. Dispatch is (1, 1, 1).
        let mut cpass = gpu.encoder.begin_compute_pass(self.name());
        cpass.set_pipeline(pipeline);
        cpass.set_bind_group(1, &bind_group);
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
    /// Cull-pass output (indirect-draw arg buffer).
    pub indirect_draws: ResourceId,
    /// G-buffer attachment: albedo (RGB) + roughness (A).
    pub gbuffer_albedo_roughness: ResourceId,
    /// G-buffer attachment: normal (RG) + metallic (B) + AO (A).
    pub gbuffer_normal_metallic: ResourceId,
    /// G-buffer attachment: motion (RG) + view-z (B).
    pub gbuffer_motion_depth: ResourceId,
    /// Hardware D32F depth (reverse-Z).
    pub depth: ResourceId,
    /// Per-frame UBO (`@group(0) @binding(0)`).
    pub frame_uniforms: ResourceId,
    pipeline: Option<RenderPipeline>,
}

impl GBufferPass {
    /// Construct with the resource handles the graph builder produced.
    pub fn new(
        indirect_draws: ResourceId,
        gbuffer_albedo_roughness: ResourceId,
        gbuffer_normal_metallic: ResourceId,
        gbuffer_motion_depth: ResourceId,
        depth: ResourceId,
        frame_uniforms: ResourceId,
    ) -> Self {
        Self {
            indirect_draws,
            gbuffer_albedo_roughness,
            gbuffer_normal_metallic,
            gbuffer_motion_depth,
            depth,
            frame_uniforms,
            pipeline: None,
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
    fn install_pipeline(&mut self, device: &Device) -> Result<(), ShaderError> {
        self.pipeline = Some(build_gbuffer_pipeline(device)?);
        Ok(())
    }
    fn record(&mut self, ctx: &mut PassContext) {
        // ADR-075 §1 — six-step template, MRT + depth render pass.
        //
        // Opens the 3-MRT + depth render pass against the G-buffer
        // attachments, clears them, sets the pipeline. Per-draw push
        // constants + vertex/index/indirect-draw consumption land in
        // A.2d (same pattern as CsmShadowPass — both consume the
        // CullPass's indirect-draw buffer).
        let Some(gpu) = ctx.gpu.as_mut() else {
            return;
        };
        let Some(pipeline) = self.pipeline.as_ref() else {
            return;
        };
        let Some(resources) = ctx.resources else {
            return;
        };
        let Some(_frame) = resources.resolve_buffer(self.frame_uniforms) else {
            return;
        };
        let Some(albedo) = resources.resolve_view(self.gbuffer_albedo_roughness) else {
            return;
        };
        let Some(normal) = resources.resolve_view(self.gbuffer_normal_metallic) else {
            return;
        };
        let Some(motion) = resources.resolve_view(self.gbuffer_motion_depth) else {
            return;
        };
        let Some(depth) = resources.resolve_view(self.depth) else {
            return;
        };
        let color_attachments = [
            RenderPassColorAttachment {
                view: &albedo,
                load: LoadOp::Clear(Color::BLACK),
                store: true,
            },
            RenderPassColorAttachment {
                view: &normal,
                load: LoadOp::Clear(Color::BLACK),
                store: true,
            },
            RenderPassColorAttachment {
                view: &motion,
                load: LoadOp::Clear(Color::BLACK),
                store: true,
            },
        ];
        let mut rpass = gpu.encoder.begin_render_pass_desc(&RenderPassDesc {
            label: "draw.opaque.renderpass",
            color_attachments: &color_attachments,
            depth: Some(RenderPassDepthAttachment {
                view: &depth,
                load: DepthLoadOp::Clear(0.0),
                store: true,
            }),
        });
        rpass.set_pipeline(pipeline);
        // A.2d: per-frame UBO bind + per-draw push constants +
        // `set_vertex_buffer` + `set_index_buffer_u32` + per-instance
        // `draw_indexed_indirect` against the cull pass's draw-arg
        // buffer. Until then the pass clears the MRT attachments.
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
    /// G-buffer albedo+roughness (`@group(2) @binding(0)`).
    pub gbuffer_albedo_roughness: ResourceId,
    /// G-buffer normal+metallic (`@group(2) @binding(1)`).
    pub gbuffer_normal_metallic: ResourceId,
    /// G-buffer motion+view-z (`@group(2) @binding(2)`).
    pub gbuffer_motion_depth: ResourceId,
    /// Hardware depth (`@group(2) @binding(3)`).
    pub depth: ResourceId,
    /// Cluster grid (`@group(1) @binding(2)`).
    pub cluster_cells: ResourceId,
    /// Per-light SSBO (`@group(1) @binding(1)`).
    pub lights: ResourceId,
    /// Shadow atlas (`@group(2) @binding(4)`).
    pub shadow_atlas: ResourceId,
    /// HDR linear-space output (color attachment).
    pub lit_color: ResourceId,
    /// Full-screen frame UBO (`@group(0) @binding(0)`).
    pub frame_uniforms: ResourceId,
    /// Cluster UBO (`@group(1) @binding(0)`).
    pub cluster_uniforms: ResourceId,
    /// Light-indices SSBO (`@group(1) @binding(3)`).
    pub light_indices: ResourceId,
    /// Shadow comparison sampler (`@group(2) @binding(5)`).
    pub shadow_sampler: ResourceId,
    pipeline: Option<RenderPipeline>,
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
        frame_uniforms: ResourceId,
        cluster_uniforms: ResourceId,
        light_indices: ResourceId,
        shadow_sampler: ResourceId,
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
            frame_uniforms,
            cluster_uniforms,
            light_indices,
            shadow_sampler,
            pipeline: None,
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
    fn install_pipeline(&mut self, device: &Device) -> Result<(), ShaderError> {
        self.pipeline = Some(build_lighting_pipeline(device)?);
        Ok(())
    }
    fn record(&mut self, ctx: &mut PassContext) {
        // ADR-075 §1 — six-step template. Full-screen draw (no vertex
        // buffer); the WGSL `vs_main` generates a 3-vertex triangle
        // via `@builtin(vertex_index)`.
        let Some(gpu) = ctx.gpu.as_mut() else {
            return;
        };
        let Some(pipeline) = self.pipeline.as_ref() else {
            return;
        };
        let Some(resources) = ctx.resources else {
            return;
        };
        let Some(frame) = resources.resolve_buffer(self.frame_uniforms) else {
            return;
        };
        let Some(cluster_u) = resources.resolve_buffer(self.cluster_uniforms) else {
            return;
        };
        let Some(lights) = resources.resolve_buffer(self.lights) else {
            return;
        };
        let Some(cells) = resources.resolve_buffer(self.cluster_cells) else {
            return;
        };
        let Some(indices) = resources.resolve_buffer(self.light_indices) else {
            return;
        };
        let Some(albedo) = resources.resolve_view(self.gbuffer_albedo_roughness) else {
            return;
        };
        let Some(normal) = resources.resolve_view(self.gbuffer_normal_metallic) else {
            return;
        };
        let Some(motion) = resources.resolve_view(self.gbuffer_motion_depth) else {
            return;
        };
        let Some(depth) = resources.resolve_view(self.depth) else {
            return;
        };
        let Some(shadow) = resources.resolve_view(self.shadow_atlas) else {
            return;
        };
        let Some(shadow_sampler) = resources.resolve_sampler(self.shadow_sampler) else {
            return;
        };
        let Some(lit) = resources.resolve_view(self.lit_color) else {
            return;
        };
        let layout_frame = pipeline.bind_group_layout(0);
        let layout_cluster = pipeline.bind_group_layout(1);
        let layout_tex = pipeline.bind_group_layout(2);
        let bg_frame = BindGroup::new(
            gpu.device,
            &BindGroupDesc {
                label: "draw.opaque.2.bindgroup.0",
                layout: &layout_frame,
                entries: &[BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::Buffer(frame),
                }],
            },
        );
        let bg_cluster = BindGroup::new(
            gpu.device,
            &BindGroupDesc {
                label: "draw.opaque.2.bindgroup.1",
                layout: &layout_cluster,
                entries: &[
                    BindGroupEntry {
                        binding: 0,
                        resource: BindingResource::Buffer(cluster_u),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: BindingResource::Buffer(lights),
                    },
                    BindGroupEntry {
                        binding: 2,
                        resource: BindingResource::Buffer(cells),
                    },
                    BindGroupEntry {
                        binding: 3,
                        resource: BindingResource::Buffer(indices),
                    },
                ],
            },
        );
        let bg_tex = BindGroup::new(
            gpu.device,
            &BindGroupDesc {
                label: "draw.opaque.2.bindgroup.2",
                layout: &layout_tex,
                entries: &[
                    BindGroupEntry {
                        binding: 0,
                        resource: BindingResource::TextureView(&albedo),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: BindingResource::TextureView(&normal),
                    },
                    BindGroupEntry {
                        binding: 2,
                        resource: BindingResource::TextureView(&motion),
                    },
                    BindGroupEntry {
                        binding: 3,
                        resource: BindingResource::TextureView(&depth),
                    },
                    BindGroupEntry {
                        binding: 4,
                        resource: BindingResource::TextureView(&shadow),
                    },
                    BindGroupEntry {
                        binding: 5,
                        resource: BindingResource::Sampler(shadow_sampler),
                    },
                ],
            },
        );
        // Open the render pass with one color attachment (lit_color),
        // no depth (the pipeline's depth_stencil is None — full-screen
        // lighting doesn't write depth).
        let color_attachments = [RenderPassColorAttachment {
            view: &lit,
            load: LoadOp::Clear(Color::BLACK),
            store: true,
        }];
        let mut rpass = gpu.encoder.begin_render_pass_desc(&RenderPassDesc {
            label: "draw.opaque.2.renderpass",
            color_attachments: &color_attachments,
            depth: None,
        });
        rpass.set_pipeline(pipeline);
        rpass.set_bind_group(0, &bg_frame);
        rpass.set_bind_group(1, &bg_cluster);
        rpass.set_bind_group(2, &bg_tex);
        // Full-screen triangle: 3 vertices, 1 instance.
        rpass.draw(0..3, 0..1);
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
    /// attachment) — `@group(2) @binding(1)` (depth texture).
    pub depth: ResourceId,
    /// G-buffer normals — `@group(2) @binding(0)`.
    pub gbuffer_normal_metallic: ResourceId,
    /// Single-channel occlusion output (storage texture,
    /// `@group(2) @binding(2)`).
    pub ssao_target: ResourceId,
    /// SSAO uniforms UBO (`@group(1) @binding(0)`).
    pub ssao_uniforms: ResourceId,
    pipeline: Option<ComputePipeline>,
}

impl SsaoPass {
    /// Construct with the resource handles the graph builder produced.
    pub fn new(
        depth: ResourceId,
        gbuffer_normal_metallic: ResourceId,
        ssao_target: ResourceId,
        ssao_uniforms: ResourceId,
    ) -> Self {
        Self {
            depth,
            gbuffer_normal_metallic,
            ssao_target,
            ssao_uniforms,
            pipeline: None,
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
    fn install_pipeline(&mut self, device: &Device) -> Result<(), ShaderError> {
        self.pipeline = Some(build_ssao_pipeline(device)?);
        Ok(())
    }
    fn record(&mut self, ctx: &mut PassContext) {
        // ADR-075 §1 — six-step template.
        let Some(gpu) = ctx.gpu.as_mut() else {
            return;
        };
        let Some(pipeline) = self.pipeline.as_ref() else {
            return;
        };
        let Some(resources) = ctx.resources else {
            return;
        };
        let Some(ssao_uniforms) = resources.resolve_buffer(self.ssao_uniforms) else {
            return;
        };
        let Some(gbuf_n) = resources.resolve_view(self.gbuffer_normal_metallic) else {
            return;
        };
        let Some(depth) = resources.resolve_view(self.depth) else {
            return;
        };
        let Some(ssao_out) = resources.resolve_view(self.ssao_target) else {
            return;
        };
        let layout_uniforms = pipeline.bind_group_layout(1);
        let layout_textures = pipeline.bind_group_layout(2);
        let bg_uniforms = BindGroup::new(
            gpu.device,
            &BindGroupDesc {
                label: "post.fx.ssao.bindgroup.1",
                layout: &layout_uniforms,
                entries: &[BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::Buffer(ssao_uniforms),
                }],
            },
        );
        let bg_textures = BindGroup::new(
            gpu.device,
            &BindGroupDesc {
                label: "post.fx.ssao.bindgroup.2",
                layout: &layout_textures,
                entries: &[
                    BindGroupEntry {
                        binding: 0,
                        resource: BindingResource::TextureView(&gbuf_n),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: BindingResource::TextureView(&depth),
                    },
                    BindGroupEntry {
                        binding: 2,
                        resource: BindingResource::TextureView(&ssao_out),
                    },
                ],
            },
        );
        // Workgroup (8, 8, 1); dispatch over the storage-texture
        // extent. The SSAO target is half-resolution per
        // `contracts::SSAO_RESOLUTION_DIVISOR`; the texture's actual
        // dimensions drive the dispatch.
        let dim = dispatch_dim_for_view(&ssao_out);
        let mut cpass = gpu.encoder.begin_compute_pass(self.name());
        cpass.set_pipeline(pipeline);
        cpass.set_bind_group(1, &bg_uniforms);
        cpass.set_bind_group(2, &bg_textures);
        cpass.dispatch_workgroups(dim.0.div_ceil(8), dim.1.div_ceil(8), 1);
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
    /// L2 SH probe set buffer (`@group(1) @binding(1)`).
    pub probes: ResourceId,
    /// 512×512 Karis split-sum BRDF LUT (`@group(2) @binding(3)`).
    pub brdf_lut: ResourceId,
    /// G-buffer albedo + roughness (`@group(2) @binding(0)`).
    pub gbuffer_albedo_roughness: ResourceId,
    /// G-buffer normal + metallic (`@group(2) @binding(1)`).
    pub gbuffer_normal_metallic: ResourceId,
    /// Hardware depth (`@group(2) @binding(2)`).
    pub depth: ResourceId,
    /// HDR linear-space output (storage texture,
    /// `@group(2) @binding(5)`).
    pub lit_color: ResourceId,
    /// IBL uniforms UBO (`@group(1) @binding(0)`).
    pub ibl_uniforms: ResourceId,
    /// BRDF LUT sampler (`@group(2) @binding(4)`).
    pub brdf_sampler: ResourceId,
    pipeline: Option<ComputePipeline>,
}

impl IblPass {
    /// Construct with the resource handles the graph builder produced.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        probes: ResourceId,
        brdf_lut: ResourceId,
        gbuffer_albedo_roughness: ResourceId,
        gbuffer_normal_metallic: ResourceId,
        depth: ResourceId,
        lit_color: ResourceId,
        ibl_uniforms: ResourceId,
        brdf_sampler: ResourceId,
    ) -> Self {
        Self {
            probes,
            brdf_lut,
            gbuffer_albedo_roughness,
            gbuffer_normal_metallic,
            depth,
            lit_color,
            ibl_uniforms,
            brdf_sampler,
            pipeline: None,
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
    fn install_pipeline(&mut self, device: &Device) -> Result<(), ShaderError> {
        self.pipeline = Some(build_ibl_evaluate_pipeline(device)?);
        Ok(())
    }
    fn record(&mut self, ctx: &mut PassContext) {
        // ADR-075 §1 — six-step template.
        let Some(gpu) = ctx.gpu.as_mut() else {
            return;
        };
        let Some(pipeline) = self.pipeline.as_ref() else {
            return;
        };
        let Some(resources) = ctx.resources else {
            return;
        };
        let Some(ibl_u) = resources.resolve_buffer(self.ibl_uniforms) else {
            return;
        };
        let Some(probes) = resources.resolve_buffer(self.probes) else {
            return;
        };
        let Some(albedo) = resources.resolve_view(self.gbuffer_albedo_roughness) else {
            return;
        };
        let Some(normal) = resources.resolve_view(self.gbuffer_normal_metallic) else {
            return;
        };
        let Some(depth) = resources.resolve_view(self.depth) else {
            return;
        };
        let Some(brdf_lut) = resources.resolve_view(self.brdf_lut) else {
            return;
        };
        let Some(sampler) = resources.resolve_sampler(self.brdf_sampler) else {
            return;
        };
        let Some(out) = resources.resolve_view(self.lit_color) else {
            return;
        };
        let layout_u = pipeline.bind_group_layout(1);
        let layout_tex = pipeline.bind_group_layout(2);
        let bg_u = BindGroup::new(
            gpu.device,
            &BindGroupDesc {
                label: "draw.opaque.ibl.bindgroup.1",
                layout: &layout_u,
                entries: &[
                    BindGroupEntry {
                        binding: 0,
                        resource: BindingResource::Buffer(ibl_u),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: BindingResource::Buffer(probes),
                    },
                ],
            },
        );
        let bg_tex = BindGroup::new(
            gpu.device,
            &BindGroupDesc {
                label: "draw.opaque.ibl.bindgroup.2",
                layout: &layout_tex,
                entries: &[
                    BindGroupEntry {
                        binding: 0,
                        resource: BindingResource::TextureView(&albedo),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: BindingResource::TextureView(&normal),
                    },
                    BindGroupEntry {
                        binding: 2,
                        resource: BindingResource::TextureView(&depth),
                    },
                    BindGroupEntry {
                        binding: 3,
                        resource: BindingResource::TextureView(&brdf_lut),
                    },
                    BindGroupEntry {
                        binding: 4,
                        resource: BindingResource::Sampler(sampler),
                    },
                    BindGroupEntry {
                        binding: 5,
                        resource: BindingResource::TextureView(&out),
                    },
                ],
            },
        );
        let dim = dispatch_dim_for_view(&out);
        let mut cpass = gpu.encoder.begin_compute_pass(self.name());
        cpass.set_pipeline(pipeline);
        cpass.set_bind_group(1, &bg_u);
        cpass.set_bind_group(2, &bg_tex);
        cpass.dispatch_workgroups(dim.0.div_ceil(8), dim.1.div_ceil(8), 1);
    }
}

// =============================================================================
// TAA resolve (PR 4).
// =============================================================================

/// TAA accumulation + history (ADR-042).
#[derive(Debug)]
pub struct TaaPass {
    /// Current-frame HDR colour (lighting accumulation output),
    /// `@group(2) @binding(0)` (`current_color`).
    pub lit_color: ResourceId,
    /// Previous-frame TAA history, `@group(2) @binding(2)`.
    pub history: ResourceId,
    /// Motion + view-z attachment from the G-buffer pass,
    /// `@group(2) @binding(3)`.
    pub gbuffer_motion_depth: ResourceId,
    /// Hardware depth (NOT bound today — the TAA shader reads depth
    /// from `gbuffer_motion_depth.z`; the field is retained to keep
    /// the graph-flow declaration honest for the upcoming disocclusion
    /// mask refinement).
    pub depth: ResourceId,
    /// TAA-resolved HDR target (also the canonical upscaler input),
    /// `@group(2) @binding(5)`.
    pub resolved: ResourceId,
    /// Next-frame history slot the pool ping-pongs into,
    /// `@group(2) @binding(6)`.
    pub history_next: ResourceId,
    /// IBL contribution input, `@group(2) @binding(1)`.
    pub ibl_contribution: ResourceId,
    /// TAA uniforms UBO, `@group(1) @binding(0)`.
    pub taa_uniforms: ResourceId,
    /// Linear sampler (`@group(2) @binding(4)`).
    pub linear_sampler: ResourceId,
    pipeline: Option<ComputePipeline>,
}

impl TaaPass {
    /// Construct with the resource handles the graph builder produced.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        lit_color: ResourceId,
        history: ResourceId,
        gbuffer_motion_depth: ResourceId,
        depth: ResourceId,
        resolved: ResourceId,
        history_next: ResourceId,
        ibl_contribution: ResourceId,
        taa_uniforms: ResourceId,
        linear_sampler: ResourceId,
    ) -> Self {
        Self {
            lit_color,
            history,
            gbuffer_motion_depth,
            depth,
            resolved,
            history_next,
            ibl_contribution,
            taa_uniforms,
            linear_sampler,
            pipeline: None,
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
    fn install_pipeline(&mut self, device: &Device) -> Result<(), ShaderError> {
        self.pipeline = Some(build_taa_resolve_pipeline(device)?);
        Ok(())
    }
    fn record(&mut self, ctx: &mut PassContext) {
        // ADR-075 §1 — six-step template.
        let Some(gpu) = ctx.gpu.as_mut() else {
            return;
        };
        let Some(pipeline) = self.pipeline.as_ref() else {
            return;
        };
        let Some(resources) = ctx.resources else {
            return;
        };
        let Some(taa_u) = resources.resolve_buffer(self.taa_uniforms) else {
            return;
        };
        let Some(curr) = resources.resolve_view(self.lit_color) else {
            return;
        };
        let Some(ibl) = resources.resolve_view(self.ibl_contribution) else {
            return;
        };
        let Some(hist) = resources.resolve_view(self.history) else {
            return;
        };
        let Some(motion) = resources.resolve_view(self.gbuffer_motion_depth) else {
            return;
        };
        let Some(sampler) = resources.resolve_sampler(self.linear_sampler) else {
            return;
        };
        let Some(resolved_out) = resources.resolve_view(self.resolved) else {
            return;
        };
        let Some(hist_out) = resources.resolve_view(self.history_next) else {
            return;
        };
        let layout_u = pipeline.bind_group_layout(1);
        let layout_tex = pipeline.bind_group_layout(2);
        let bg_u = BindGroup::new(
            gpu.device,
            &BindGroupDesc {
                label: "post.fx.taa.bindgroup.1",
                layout: &layout_u,
                entries: &[BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::Buffer(taa_u),
                }],
            },
        );
        let bg_tex = BindGroup::new(
            gpu.device,
            &BindGroupDesc {
                label: "post.fx.taa.bindgroup.2",
                layout: &layout_tex,
                entries: &[
                    BindGroupEntry {
                        binding: 0,
                        resource: BindingResource::TextureView(&curr),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: BindingResource::TextureView(&ibl),
                    },
                    BindGroupEntry {
                        binding: 2,
                        resource: BindingResource::TextureView(&hist),
                    },
                    BindGroupEntry {
                        binding: 3,
                        resource: BindingResource::TextureView(&motion),
                    },
                    BindGroupEntry {
                        binding: 4,
                        resource: BindingResource::Sampler(sampler),
                    },
                    BindGroupEntry {
                        binding: 5,
                        resource: BindingResource::TextureView(&resolved_out),
                    },
                    BindGroupEntry {
                        binding: 6,
                        resource: BindingResource::TextureView(&hist_out),
                    },
                ],
            },
        );
        let dim = dispatch_dim_for_view(&resolved_out);
        let mut cpass = gpu.encoder.begin_compute_pass(self.name());
        cpass.set_pipeline(pipeline);
        cpass.set_bind_group(1, &bg_u);
        cpass.set_bind_group(2, &bg_tex);
        cpass.dispatch_workgroups(dim.0.div_ceil(8), dim.1.div_ceil(8), 1);
    }
}

// =============================================================================
// Bloom (PR 4) — three compute pipelines: extract, downsample, upsample.
// =============================================================================

/// Bloom extract + blur (PR 4). Reads the TAA-resolved HDR target;
/// writes the low-frequency bright-pass layer for the tonemap pass to
/// composite.
///
/// Phase 5.5 A.2d ships the full mip chain (extract + 4 downsample +
/// 4 upsample = 9 dispatches per frame). The bloom target texture is
/// allocated by the host renderer with
/// [`contracts::BLOOM_MIP_LEVELS`] mip levels (5 total); each mip is
/// bound independently via [`engine_gpu::Texture::mip_view`]. The
/// final composite lands in mip 0 which TonemapPass samples (its
/// `textureSampleLevel(bloom, _, _, 0.0)` reads the level explicitly).
///
/// The kernel is a Jimenez-2014 dual-filter Kawase blur (per ADR-065
/// §5); ADR-046's 1/255 channel + p99 ≤ 1% tolerance absorbs the
/// kernel-shape difference vs the CPU oracle's `gaussian_blur_3x3`.
#[derive(Debug)]
pub struct BloomPass {
    /// TAA-resolved HDR input (`@group(2) @binding(0)`).
    pub resolved: ResourceId,
    /// Bloom layer output — a mip-chain texture allocated with
    /// [`contracts::BLOOM_MIP_LEVELS`] mip levels. Mip 0 is the final
    /// composite that downstream passes (Tonemap) read.
    pub bloom_target: ResourceId,
    /// Bloom uniforms UBO (`@group(1) @binding(0)`).
    pub bloom_uniforms: ResourceId,
    /// Linear sampler (`@group(2) @binding(1)`).
    pub linear_sampler: ResourceId,
    pipeline_extract: Option<ComputePipeline>,
    pipeline_downsample: Option<ComputePipeline>,
    pipeline_upsample: Option<ComputePipeline>,
}

impl BloomPass {
    /// Construct with the resource handles the graph builder produced.
    pub fn new(
        resolved: ResourceId,
        bloom_target: ResourceId,
        bloom_uniforms: ResourceId,
        linear_sampler: ResourceId,
    ) -> Self {
        Self {
            resolved,
            bloom_target,
            bloom_uniforms,
            linear_sampler,
            pipeline_extract: None,
            pipeline_downsample: None,
            pipeline_upsample: None,
        }
    }
}

/// One stage of the bloom mip chain: `(pipeline, src, dst)` plus a
/// debug label. The record() body builds one bind group + one
/// dispatch per stage.
struct BloomStage<'a> {
    label: &'a str,
    pipeline: &'a ComputePipeline,
    src: engine_gpu::TextureView<'a>,
    dst: engine_gpu::TextureView<'a>,
}

fn dispatch_bloom_stage(
    encoder: &mut engine_gpu::CommandEncoder,
    device: &Device,
    stage: &BloomStage<'_>,
    bloom_u: &engine_gpu::Buffer,
    sampler: &engine_gpu::Sampler,
) {
    let layout_u = stage.pipeline.bind_group_layout(1);
    let layout_tex = stage.pipeline.bind_group_layout(2);
    let bg_u = BindGroup::new(
        device,
        &BindGroupDesc {
            label: stage.label,
            layout: &layout_u,
            entries: &[BindGroupEntry {
                binding: 0,
                resource: BindingResource::Buffer(bloom_u),
            }],
        },
    );
    let bg_tex = BindGroup::new(
        device,
        &BindGroupDesc {
            label: stage.label,
            layout: &layout_tex,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::TextureView(&stage.src),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::Sampler(sampler),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: BindingResource::TextureView(&stage.dst),
                },
            ],
        },
    );
    let dim = dispatch_dim_for_view(&stage.dst);
    let mut cpass = encoder.begin_compute_pass(stage.label);
    cpass.set_pipeline(stage.pipeline);
    cpass.set_bind_group(1, &bg_u);
    cpass.set_bind_group(2, &bg_tex);
    cpass.dispatch_workgroups(dim.0.div_ceil(8), dim.1.div_ceil(8), 1);
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
    fn install_pipeline(&mut self, device: &Device) -> Result<(), ShaderError> {
        self.pipeline_extract = Some(build_bloom_extract_pipeline(device)?);
        self.pipeline_downsample = Some(build_bloom_downsample_pipeline(device)?);
        self.pipeline_upsample = Some(build_bloom_upsample_pipeline(device)?);
        Ok(())
    }
    fn record(&mut self, ctx: &mut PassContext) {
        // ADR-075 §1 — six-step template. ADR-065 §5 mip chain: 1
        // extract + 4 downsample + 4 upsample = 9 dispatches.
        let Some(gpu) = ctx.gpu.as_mut() else {
            return;
        };
        let Some(extract) = self.pipeline_extract.as_ref() else {
            return;
        };
        let Some(downsample) = self.pipeline_downsample.as_ref() else {
            return;
        };
        let Some(upsample) = self.pipeline_upsample.as_ref() else {
            return;
        };
        let Some(resources) = ctx.resources else {
            return;
        };
        let Some(bloom_u) = resources.resolve_buffer(self.bloom_uniforms) else {
            return;
        };
        let Some(resolved_view) = resources.resolve_view(self.resolved) else {
            return;
        };
        let Some(sampler) = resources.resolve_sampler(self.linear_sampler) else {
            return;
        };
        // The bloom mip chain reads/writes individual mip levels of a
        // single bloom_target texture. Each mip needs its own view —
        // both because storage-texture writes accept only
        // `mip_level_count = 1` views, and because sampling needs to
        // target a specific mip. Acquire per-mip views from the
        // resolved texture handle (Phase 5.5 A.2d added
        // `engine_gpu::Texture::mip_view`). The resolver's
        // resolve_view returns the default view; we need direct
        // texture access.
        //
        // For A.2d, the resolver doesn't yet expose `resolve_texture`,
        // so we use the resolved view's owning texture by going
        // through a separate `resolve_texture` accessor on the
        // resolver. (Added in this commit alongside.)
        let Some(bloom_tex) = resources.resolve_texture(self.bloom_target) else {
            return;
        };
        let mip_count = bloom_tex.mip_level_count();
        if mip_count < 2 {
            // No mip chain to traverse; degrade gracefully (e.g.,
            // unit-test path with a single-mip target).
            return;
        }
        // Extract: resolved → mip 0.
        dispatch_bloom_stage(
            gpu.encoder,
            gpu.device,
            &BloomStage {
                label: "post.fx.bloom.extract",
                pipeline: extract,
                src: resolved_view,
                dst: bloom_tex.mip_view(0),
            },
            bloom_u,
            sampler,
        );
        // Downsample chain: mip i → mip i+1.
        for i in 0..mip_count - 1 {
            dispatch_bloom_stage(
                gpu.encoder,
                gpu.device,
                &BloomStage {
                    label: "post.fx.bloom.downsample",
                    pipeline: downsample,
                    src: bloom_tex.mip_view(i),
                    dst: bloom_tex.mip_view(i + 1),
                },
                bloom_u,
                sampler,
            );
        }
        // Upsample chain (additive blend): mip i+1 → mip i, in
        // reverse. The final composite lands at mip 0 — what
        // TonemapPass samples via `textureSampleLevel(_, _, _, 0.0)`.
        for i in (0..mip_count - 1).rev() {
            dispatch_bloom_stage(
                gpu.encoder,
                gpu.device,
                &BloomStage {
                    label: "post.fx.bloom.upsample",
                    pipeline: upsample,
                    src: bloom_tex.mip_view(i + 1),
                    dst: bloom_tex.mip_view(i),
                },
                bloom_u,
                sampler,
            );
        }
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
    /// TAA-resolved HDR input (`@group(2) @binding(0)`).
    pub resolved: ResourceId,
    /// Bloom layer (`@group(2) @binding(1)`).
    pub bloom: ResourceId,
    /// LDR output (storage texture, `@group(2) @binding(3)`).
    pub tonemapped: ResourceId,
    /// Tonemap uniforms UBO (`@group(1) @binding(0)`).
    pub tonemap_uniforms: ResourceId,
    /// Linear sampler (`@group(2) @binding(2)`).
    pub linear_sampler: ResourceId,
    pipeline: Option<ComputePipeline>,
}

impl TonemapPass {
    /// Construct with the resource handles the graph builder produced.
    pub fn new(
        resolved: ResourceId,
        bloom: ResourceId,
        tonemapped: ResourceId,
        tonemap_uniforms: ResourceId,
        linear_sampler: ResourceId,
    ) -> Self {
        Self {
            resolved,
            bloom,
            tonemapped,
            tonemap_uniforms,
            linear_sampler,
            pipeline: None,
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
    fn install_pipeline(&mut self, device: &Device) -> Result<(), ShaderError> {
        self.pipeline = Some(build_tonemap_pipeline(device)?);
        Ok(())
    }
    fn record(&mut self, ctx: &mut PassContext) {
        // ADR-075 §1 — six-step template.
        let Some(gpu) = ctx.gpu.as_mut() else {
            return;
        };
        let Some(pipeline) = self.pipeline.as_ref() else {
            return;
        };
        let Some(resources) = ctx.resources else {
            return;
        };
        let Some(tonemap_u) = resources.resolve_buffer(self.tonemap_uniforms) else {
            return;
        };
        let Some(scene) = resources.resolve_view(self.resolved) else {
            return;
        };
        let Some(bloom) = resources.resolve_view(self.bloom) else {
            return;
        };
        let Some(sampler) = resources.resolve_sampler(self.linear_sampler) else {
            return;
        };
        let Some(dst) = resources.resolve_view(self.tonemapped) else {
            return;
        };
        let layout_u = pipeline.bind_group_layout(1);
        let layout_tex = pipeline.bind_group_layout(2);
        let bg_u = BindGroup::new(
            gpu.device,
            &BindGroupDesc {
                label: "post.fx.tonemap.bindgroup.1",
                layout: &layout_u,
                entries: &[BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::Buffer(tonemap_u),
                }],
            },
        );
        let bg_tex = BindGroup::new(
            gpu.device,
            &BindGroupDesc {
                label: "post.fx.tonemap.bindgroup.2",
                layout: &layout_tex,
                entries: &[
                    BindGroupEntry {
                        binding: 0,
                        resource: BindingResource::TextureView(&scene),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: BindingResource::TextureView(&bloom),
                    },
                    BindGroupEntry {
                        binding: 2,
                        resource: BindingResource::Sampler(sampler),
                    },
                    BindGroupEntry {
                        binding: 3,
                        resource: BindingResource::TextureView(&dst),
                    },
                ],
            },
        );
        let dim = dispatch_dim_for_view(&dst);
        let mut cpass = gpu.encoder.begin_compute_pass(self.name());
        cpass.set_pipeline(pipeline);
        cpass.set_bind_group(1, &bg_u);
        cpass.set_bind_group(2, &bg_tex);
        cpass.dispatch_workgroups(dim.0.div_ceil(8), dim.1.div_ceil(8), 1);
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
        // Auxiliary non-graph-flow bindings (UBOs, samplers, secondary
        // SSBOs). These don't participate in topology — they're
        // per-pass uniform state the resolver provides at execute
        // time. Scheduling tests never resolve them, so the test only
        // needs distinct stable ResourceIds here.
        let frustum_ubo = ResourceId(50);
        let meshes_ssbo = ResourceId(51);
        let draw_count_ssbo = ResourceId(52);
        let cluster_ubo = ResourceId(53);
        let light_indices_ssbo = ResourceId(54);
        let indices_cursor_ssbo = ResourceId(55);
        // Render-pass auxiliary bindings (A.2c).
        let csm_ubo = ResourceId(63);
        let frame_ubo = ResourceId(64);
        let shadow_sampler = ResourceId(65);

        g.add_pass(CullPass::new(
            queue,
            indirect,
            frustum_ubo,
            meshes_ssbo,
            draw_count_ssbo,
        ));
        g.add_pass(CsmShadowPass::new(casters, shadow_atlas, csm_ubo));
        g.add_pass(ClusterLightPass::new(
            lights,
            cluster_cells,
            cluster_ubo,
            light_indices_ssbo,
            indices_cursor_ssbo,
        ));
        g.add_pass(GBufferPass::new(
            indirect, gbuf_ar, gbuf_nm, gbuf_md, depth, frame_ubo,
        ));
        g.add_pass(LightingAccumulationPass::new(
            gbuf_ar,
            gbuf_nm,
            gbuf_md,
            depth,
            cluster_cells,
            lights,
            shadow_atlas,
            lit,
            frame_ubo,
            cluster_ubo,
            light_indices_ssbo,
            shadow_sampler,
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
        // Auxiliary non-graph-flow bindings (UBOs, samplers, secondary
        // SSBOs / storage textures). Distinct stable ids for the
        // scheduling test; never resolved against a real resolver.
        let frustum_ubo = ResourceId(50);
        let meshes_ssbo = ResourceId(51);
        let draw_count_ssbo = ResourceId(52);
        let cluster_ubo = ResourceId(53);
        let light_indices_ssbo = ResourceId(54);
        let indices_cursor_ssbo = ResourceId(55);
        let ssao_ubo = ResourceId(56);
        // Auxiliary IDs for IBL / TAA / Bloom / Tonemap bindings.
        let ibl_ubo = ResourceId(57);
        let brdf_sampler = ResourceId(58);
        let taa_ubo = ResourceId(59);
        let linear_sampler = ResourceId(60);
        let bloom_ubo = ResourceId(61);
        let tonemap_ubo = ResourceId(62);
        // Render-pass auxiliary bindings (A.2c).
        let csm_ubo = ResourceId(63);
        let frame_ubo = ResourceId(64);
        let shadow_sampler = ResourceId(65);

        g.add_pass(CullPass::new(
            queue,
            indirect,
            frustum_ubo,
            meshes_ssbo,
            draw_count_ssbo,
        ));
        g.add_pass(CsmShadowPass::new(casters, shadow_atlas, csm_ubo));
        g.add_pass(ClusterLightPass::new(
            lights,
            cluster_cells,
            cluster_ubo,
            light_indices_ssbo,
            indices_cursor_ssbo,
        ));
        g.add_pass(GBufferPass::new(
            indirect, gbuf_ar, gbuf_nm, gbuf_md, depth, frame_ubo,
        ));
        g.add_pass(SsaoPass::new(depth, gbuf_nm, ssao, ssao_ubo));
        g.add_pass(IblPass::new(
            probes,
            brdf_lut,
            gbuf_ar,
            gbuf_nm,
            depth,
            lit,
            ibl_ubo,
            brdf_sampler,
        ));
        g.add_pass(LightingAccumulationPass::new(
            gbuf_ar,
            gbuf_nm,
            gbuf_md,
            depth,
            cluster_cells,
            lights,
            shadow_atlas,
            lit,
            frame_ubo,
            cluster_ubo,
            light_indices_ssbo,
            shadow_sampler,
        ));
        g.add_pass(TaaPass::new(
            lit,
            taa_history_prev,
            gbuf_md,
            depth,
            taa_resolved,
            taa_history_next,
            lit,
            taa_ubo,
            linear_sampler,
        ));
        g.add_pass(BloomPass::new(
            taa_resolved,
            bloom,
            bloom_ubo,
            linear_sampler,
        ));
        g.add_pass(TonemapPass::new(
            taa_resolved,
            bloom,
            tonemapped,
            tonemap_ubo,
            linear_sampler,
        ));
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
        let frustum_ubo = ResourceId(50);
        let meshes_ssbo = ResourceId(51);
        let draw_count_ssbo = ResourceId(52);
        let cluster_ubo = ResourceId(53);
        let light_indices_ssbo = ResourceId(54);
        let indices_cursor_ssbo = ResourceId(55);
        let taa_ubo = ResourceId(59);
        let linear_sampler = ResourceId(60);
        let bloom_ubo = ResourceId(61);
        let tonemap_ubo = ResourceId(62);
        let csm_ubo = ResourceId(63);
        let frame_ubo = ResourceId(64);
        let shadow_sampler = ResourceId(65);

        g.add_pass(CullPass::new(
            queue,
            indirect,
            frustum_ubo,
            meshes_ssbo,
            draw_count_ssbo,
        ));
        g.add_pass(CsmShadowPass::new(casters, shadow_atlas, csm_ubo));
        g.add_pass(ClusterLightPass::new(
            lights,
            cluster_cells,
            cluster_ubo,
            light_indices_ssbo,
            indices_cursor_ssbo,
        ));
        g.add_pass(GBufferPass::new(
            indirect, gbuf_ar, gbuf_nm, gbuf_md, depth, frame_ubo,
        ));
        g.add_pass(LightingAccumulationPass::new(
            gbuf_ar,
            gbuf_nm,
            gbuf_md,
            depth,
            cluster_cells,
            lights,
            shadow_atlas,
            lit,
            frame_ubo,
            cluster_ubo,
            light_indices_ssbo,
            shadow_sampler,
        ));
        g.add_pass(TaaPass::new(
            lit,
            taa_history_prev,
            gbuf_md,
            depth,
            taa_resolved,
            taa_history_next,
            lit,
            taa_ubo,
            linear_sampler,
        ));
        g.add_pass(UpscalePass::new(taa_resolved, upscaled));
        g.add_pass(BloomPass::new(
            taa_resolved,
            bloom,
            bloom_ubo,
            linear_sampler,
        ));
        g.add_pass(TonemapPass::new(
            upscaled,
            bloom,
            tonemapped,
            tonemap_ubo,
            linear_sampler,
        ));

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
