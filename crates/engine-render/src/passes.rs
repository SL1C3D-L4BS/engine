//! Phase 5 PR 3 — five deferred render-graph passes.
//!
//! All five implementations name [`engine_gpu`] types; none names
//! `wgpu` directly (ADR-049). Each pass's `record()` body would, on a
//! real device, encode the relevant draw / dispatch commands. PR 3's
//! milestone is the *scheduling contract*: the graph compiles the
//! correct producer/consumer order, the trait bodies own pipeline
//! setup, and the CPU oracle in `engine-raster` validates the math
//! (see ADR-046 verification sections of ADR-040 + ADR-043).
//!
//! Pass scheduling order produced by `RenderGraph::compile()`:
//!
//! 1. [`CullPass`]    — reads `RenderQueue`, writes `IndirectDrawBuffer`.
//! 2. [`CsmShadowPass`] — reads `ShadowCasters`, writes `ShadowAtlas`.
//! 3. [`ClusterLightPass`] — reads `LightSsbo`, writes `ClusterCells`.
//! 4. [`GBufferPass`] — reads `IndirectDrawBuffer`, writes the three
//!    G-buffer attachments + `DepthBuffer`.
//! 5. [`LightingAccumulationPass`] — reads `GBufferAlbedoRoughness`,
//!    `GBufferNormalMetallic`, `GBufferMotionDepth`, `DepthBuffer`,
//!    `ClusterCells`, `LightSsbo`, `ShadowAtlas`; writes `LitColor`.

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
}
