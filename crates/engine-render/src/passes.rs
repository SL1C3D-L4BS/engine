//! Phase 5 PR 3 + PR 4 — ten deferred render-graph passes.
//!
//! All implementations name [`engine_gpu`] types; none names `wgpu`
//! directly (ADR-049). Each pass's `record()` body would, on a real
//! device, encode the relevant draw / dispatch commands. PR 3 + PR 4's
//! milestone is the *scheduling contract*: the graph compiles the
//! correct producer/consumer order, the trait bodies own pipeline
//! setup, and the CPU oracle in `engine-raster` validates the math
//! (see ADR-046 verification sections of ADR-040 + ADR-041 + ADR-042 +
//! ADR-043).
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

use crate::render_graph::{Pass, PassContext, ResourceId, ResourceSet, Track};

/// Front-end frustum + occlusion culling. PR 3 lands the frustum-only
/// path; the occlusion query feedback channel is a Phase 6+ follow-up.
#[derive(Debug, Clone, Copy)]
pub struct CullPass {
    /// Graph handle for the input render queue.
    pub render_queue: ResourceId,
    /// Graph handle for the output indirect-draw buffer.
    pub indirect_draws: ResourceId,
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
    fn record(&mut self, _ctx: &mut PassContext) {
        // Compute dispatch: per-instance frustum test, append to
        // `indirect_draws`. The GPU implementation lands as a
        // workgroup-per-cluster kernel; the CPU oracle in
        // `engine-raster::scene::Frustum::rejects_aabb` is the
        // reference. Phase-5 PR 3 wires only the graph contract.
    }
}

/// 4-cascade CSM (ADR-040). One dispatch per cascade; each renders the
/// `ShadowCasters` queue into its quadrant of the 4096² atlas.
#[derive(Debug, Clone, Copy)]
pub struct CsmShadowPass {
    /// Per-shadow-caster instance queue.
    pub shadow_casters: ResourceId,
    /// 4096² D32F shadow atlas.
    pub shadow_atlas: ResourceId,
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
    fn record(&mut self, _ctx: &mut PassContext) {
        // For each cascade `i` in 0..4:
        //   1. Compute the cascade VP via the same math as
        //      `engine_raster::shadow::build_cascades` (the oracle).
        //   2. Begin a depth-only render pass against the cascade's
        //      atlas quadrant.
        //   3. Issue the shadow-caster draws with the cascade VP
        //      bound.
        // Reverse-Z (ADR-040 §3): atlas init = 0.0, depth test is
        // GREATER. Vogel-disk PCF runs in the lighting pass.
    }
}

/// Compute-shader cluster-light assignment. 144 workgroups, 64 threads
/// each (ADR-043 §4); each workgroup walks the 24-slice depth column.
#[derive(Debug, Clone, Copy)]
pub struct ClusterLightPass {
    /// Per-light SSBO (input).
    pub lights: ResourceId,
    /// Cluster-cell SSBO (output).
    pub cluster_cells: ResourceId,
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
    fn record(&mut self, _ctx: &mut PassContext) {
        // Begin a compute pass; bind the compiled cluster-assignment
        // pipeline; dispatch `(CLUSTER_TILES_X, CLUSTER_TILES_Y, 1)`.
        // The shader performs sphere/AABB cluster intersection and
        // emits 16-bit light indices into `cluster_cells`. Overflow
        // increments the `render.cluster_light_overflow` counter
        // (ADR-043 §4).
        //
        // The CPU oracle implementation lives in
        // `engine_raster::cluster::assign_lights` and produces
        // set-identical per-cell light lists. Order of indices within
        // a cell may differ between CPU + GPU.
    }
}

/// Deferred MRT G-buffer pass (`draw.opaque`). Writes
/// albedo+roughness, normal+metallic, motion+depth, plus the hardware
/// depth attachment.
#[derive(Debug, Clone, Copy)]
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
    fn record(&mut self, _ctx: &mut PassContext) {
        // Begin a 3-MRT colour + 1-depth render pass. Bind the
        // bindless heap's descriptor set (ADR-044 §6). Issue
        // `draw_indexed_indirect(indirect_draws)`.
    }
}

/// Lighting accumulation (`draw.opaque.2`). Reads the G-buffer +
/// cluster + light SSBO + shadow atlas; runs Cook-Torrance per light
/// per pixel; writes to `LitColor`. IBL + post-FX land in PR 4.
#[derive(Debug, Clone, Copy)]
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
    fn record(&mut self, _ctx: &mut PassContext) {
        // Full-screen pass that, per fragment:
        //   1. Reconstructs world-space position from `depth` + the
        //      camera's inverse VP.
        //   2. Looks up the cluster cell via
        //      `cluster_cells[slice * CLUSTER_TILES_XY + ty * X + tx]`.
        //   3. Iterates `cell.light_indices[..cell.light_count]`,
        //      dereferences `lights[idx]`, samples the shadow atlas
        //      via Vogel-disk PCF (ADR-040 §4), and accumulates
        //      Cook-Torrance.
        //
        // The CPU oracle in `engine_raster::shading::accumulate_lighting`
        // is the reference (ADR-046 cluster_pixel_parity test).
    }
}

/// Screen-space ambient-occlusion pass (PR 4). Reads view-space depth +
/// G-buffer normals; writes a single-channel occlusion factor sampled
/// by the lighting + tonemap stages. The CPU oracle is
/// `engine_raster::post_fx::ssao_factor`.
#[derive(Debug, Clone, Copy)]
pub struct SsaoPass {
    /// View-space depth (read from the G-buffer or the hardware
    /// attachment).
    pub depth: ResourceId,
    /// G-buffer normals (RG channels carry the octahedral normal).
    pub gbuffer_normal_metallic: ResourceId,
    /// Single-channel occlusion output.
    pub ssao_target: ResourceId,
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
    fn record(&mut self, _ctx: &mut PassContext) {
        // Compute or full-screen pass: per-pixel hemisphere occlusion
        // sampler against the depth buffer; clamp-to-edge addressing
        // for out-of-bounds. The CPU reference uses the fixed 8-tap
        // Fibonacci kernel in `engine_raster::post_fx`; the GPU
        // implementation may use 16/24 taps and noise rotation, with
        // the bilateral blur as a follow-up post-FX pass.
    }
}

/// IBL diffuse + specular accumulation (ADR-041). Reads the L2 SH
/// probe set + the BRDF LUT + the G-buffer; writes the HDR colour
/// target with the IBL contribution. The lighting accumulation pass
/// (`draw.opaque.2`) runs after and adds the direct-light contribution
/// on top.
#[derive(Debug, Clone, Copy)]
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
    fn record(&mut self, _ctx: &mut PassContext) {
        // Full-screen pass that, per fragment:
        //   1. Reconstructs world-space position from `depth`.
        //   2. Trilinearly interpolates the 8-neighbour SH probes at
        //      that position (ADR-041 §3).
        //   3. Convolves the L2 SH coefficients with the surface
        //      normal cosine lobe (Ramamoorthi-Hanrahan).
        //   4. Samples the prefiltered specular cube at
        //      reflect(view, normal) with the roughness-indexed mip
        //      level + scales by the BRDF LUT (Karis split-sum,
        //      ADR-041 §4).
        //   5. Writes the IBL contribution to `lit_color`.
        //
        // The CPU oracle in `engine_raster::ibl::IblProbeSet::sample`
        // is the diffuse reference; the specular split-sum CPU oracle
        // lands with the offline bake-tool follow-up.
    }
}

/// TAA accumulation + history (ADR-042). The pass reads the lit HDR
/// target produced by the lighting accumulation, the previous-frame
/// history (double-buffered by the resource pool), and the motion +
/// depth attachments; it writes the resolved HDR target and the
/// next-frame history slot.
#[derive(Debug, Clone, Copy)]
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
    fn record(&mut self, _ctx: &mut PassContext) {
        // Full-screen pass that, per pixel:
        //   1. Reprojects the history sample at
        //      `current_pos - motion_vector`.
        //   2. Computes the 3×3 YCgCo neighbourhood AABB of the
        //      current frame; clips the history sample to it
        //      (ADR-042 §3).
        //   3. Computes the depth ratio for the disocclusion mask
        //      (`depth_ratio > 1.1 ⇒ reject`); blends in the variance
        //      and velocity-aware sharpening contributions.
        //   4. Exponentially blends the clipped history with the
        //      current sample at α ∈ [0.05, 0.5].
        //
        // The CPU reference is `engine_raster::post_fx::taa_resolve`;
        // jitter is queried per frame via
        // `engine_raster::post_fx::jitter_for_frame(ctx.frame_idx)` and
        // the shadow pass cross-references the same value (ADR-040 §5).
    }
}

/// Bloom extract + blur (PR 4). Reads the TAA-resolved HDR target;
/// writes the low-frequency bright-pass layer for the tonemap pass to
/// composite.
#[derive(Debug, Clone, Copy)]
pub struct BloomPass {
    /// TAA-resolved HDR input.
    pub resolved: ResourceId,
    /// Bloom layer output.
    pub bloom_target: ResourceId,
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
    fn record(&mut self, _ctx: &mut PassContext) {
        // Down-sample chain (typically 5–7 halvings) + bilateral
        // up-sample composite. CPU oracle:
        // `engine_raster::post_fx::bloom_extract` +
        // `engine_raster::post_fx::gaussian_blur_3x3`. The GPU uses
        // a Jimenez-2014 dual-filter Kawase blur; the 1/255 oracle
        // threshold absorbs the kernel-shape difference.
    }
}

/// Upscale pass (PR 5, ADR-005 + ADR-053). Reads the TAA-resolved HDR
/// target; writes the upscaled HDR target at the display resolution.
/// The trait surface that picks the algorithm lives in
/// [`crate::upscale::UpscalerRegistry`]; the pass body is the graph
/// adapter that the renderer / oracle / bench drive.
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
        // Resolves the active [`crate::upscale::UpscalerProvider`] via
        // the renderer's [`crate::upscale::UpscalerRegistry`]; builds
        // an [`crate::upscale::UpscaleCtx`] pointing at the input /
        // output textures and the current-frame jitter; dispatches
        // through `provider.upscale(&mut ctx)`. The CPU oracle in
        // `engine_raster::upscale::bilinear_upscale` is the reference
        // for the bilinear placeholder; vendor SDK dispatch is the
        // Phase 6 follow-up.
    }
}

/// Tonemap + bloom composite (PR 4). Reads the TAA-resolved HDR + the
/// bloom layer; writes the final LDR target (`TonemappedColor`).
#[derive(Debug, Clone, Copy)]
pub struct TonemapPass {
    /// TAA-resolved HDR input.
    pub resolved: ResourceId,
    /// Bloom layer.
    pub bloom: ResourceId,
    /// LDR output.
    pub tonemapped: ResourceId,
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
    fn record(&mut self, _ctx: &mut PassContext) {
        // Composites the bloom layer additively into the HDR target,
        // then applies the ACES filmic tonemap curve
        // (`engine_raster::post_fx::tonemap_aces`) and stores the
        // sRGB-encoded LDR output. The optional CA / Vignette / Grain
        // grade lands as a follow-up; the upscaler in PR 5 consumes
        // the TAA-resolved target, not this one (TAA → Upscale →
        // Tonemap is the upscaler-path variant). Track A's no-upscale
        // path is what PR 4 wires.
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
        // Resource ids — densely numbered, the future graph builder
        // hands these out.
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

        g.add_pass(CullPass {
            render_queue: queue,
            indirect_draws: indirect,
        });
        g.add_pass(CsmShadowPass {
            shadow_casters: casters,
            shadow_atlas,
        });
        g.add_pass(ClusterLightPass {
            lights,
            cluster_cells,
        });
        g.add_pass(GBufferPass {
            indirect_draws: indirect,
            gbuffer_albedo_roughness: gbuf_ar,
            gbuffer_normal_metallic: gbuf_nm,
            gbuffer_motion_depth: gbuf_md,
            depth,
        });
        g.add_pass(LightingAccumulationPass {
            gbuffer_albedo_roughness: gbuf_ar,
            gbuffer_normal_metallic: gbuf_nm,
            gbuffer_motion_depth: gbuf_md,
            depth,
            cluster_cells,
            lights,
            shadow_atlas,
            lit_color: lit,
        });
        let n = g.compile().expect("graph compiles");
        assert_eq!(n, 5);
        let names = g.scheduled_names().unwrap();
        // Cull and shadow + cluster are all independent of each other
        // but share earlier registration than draw.opaque, so the
        // registration tie-break pins them up front.
        assert_eq!(names[0], "cull");
        // draw.opaque must follow cull (write→read on indirect_draws).
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
        // PR-3 resources.
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
        // PR-4 resources.
        let probes = ResourceId(11);
        let brdf_lut = ResourceId(12);
        let ssao = ResourceId(13);
        let taa_history_prev = ResourceId(14);
        let taa_history_next = ResourceId(15);
        let taa_resolved = ResourceId(16);
        let bloom = ResourceId(17);
        let tonemapped = ResourceId(18);

        g.add_pass(CullPass {
            render_queue: queue,
            indirect_draws: indirect,
        });
        g.add_pass(CsmShadowPass {
            shadow_casters: casters,
            shadow_atlas,
        });
        g.add_pass(ClusterLightPass {
            lights,
            cluster_cells,
        });
        g.add_pass(GBufferPass {
            indirect_draws: indirect,
            gbuffer_albedo_roughness: gbuf_ar,
            gbuffer_normal_metallic: gbuf_nm,
            gbuffer_motion_depth: gbuf_md,
            depth,
        });
        g.add_pass(SsaoPass {
            depth,
            gbuffer_normal_metallic: gbuf_nm,
            ssao_target: ssao,
        });
        g.add_pass(IblPass {
            probes,
            brdf_lut,
            gbuffer_albedo_roughness: gbuf_ar,
            gbuffer_normal_metallic: gbuf_nm,
            depth,
            lit_color: lit,
        });
        g.add_pass(LightingAccumulationPass {
            gbuffer_albedo_roughness: gbuf_ar,
            gbuffer_normal_metallic: gbuf_nm,
            gbuffer_motion_depth: gbuf_md,
            depth,
            cluster_cells,
            lights,
            shadow_atlas,
            lit_color: lit,
        });
        g.add_pass(TaaPass {
            lit_color: lit,
            history: taa_history_prev,
            gbuffer_motion_depth: gbuf_md,
            depth,
            resolved: taa_resolved,
            history_next: taa_history_next,
        });
        g.add_pass(BloomPass {
            resolved: taa_resolved,
            bloom_target: bloom,
        });
        g.add_pass(TonemapPass {
            resolved: taa_resolved,
            bloom,
            tonemapped,
        });
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
        // SSAO + IBL must follow the G-buffer fill (they consume it).
        assert!(gbuf_idx < ssao_idx, "g-buffer before ssao");
        assert!(gbuf_idx < ibl_idx, "g-buffer before ibl");
        // TAA depends on lit color → lighting and IBL both come first.
        assert!(lighting_idx < taa_idx, "lighting before taa");
        assert!(ibl_idx < taa_idx, "ibl before taa");
        // Bloom + Tonemap form the tail of the post chain.
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

        g.add_pass(CullPass {
            render_queue: queue,
            indirect_draws: indirect,
        });
        g.add_pass(CsmShadowPass {
            shadow_casters: casters,
            shadow_atlas,
        });
        g.add_pass(ClusterLightPass {
            lights,
            cluster_cells,
        });
        g.add_pass(GBufferPass {
            indirect_draws: indirect,
            gbuffer_albedo_roughness: gbuf_ar,
            gbuffer_normal_metallic: gbuf_nm,
            gbuffer_motion_depth: gbuf_md,
            depth,
        });
        g.add_pass(LightingAccumulationPass {
            gbuffer_albedo_roughness: gbuf_ar,
            gbuffer_normal_metallic: gbuf_nm,
            gbuffer_motion_depth: gbuf_md,
            depth,
            cluster_cells,
            lights,
            shadow_atlas,
            lit_color: lit,
        });
        g.add_pass(TaaPass {
            lit_color: lit,
            history: taa_history_prev,
            gbuffer_motion_depth: gbuf_md,
            depth,
            resolved: taa_resolved,
            history_next: taa_history_next,
        });
        g.add_pass(UpscalePass {
            resolved: taa_resolved,
            upscaled,
        });
        g.add_pass(BloomPass {
            resolved: taa_resolved,
            bloom_target: bloom,
        });
        g.add_pass(TonemapPass {
            resolved: upscaled,
            bloom,
            tonemapped,
        });

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
