//! Init-time GPU helpers — work that runs once at renderer startup,
//! not per frame, and therefore lives outside the [`crate::render_graph`].
//!
//! Today the only resident is the BRDF LUT bake (ADR-065 §3). The
//! Karis split-sum BRDF is precomputed into a 512² Rgba16Float
//! texture and sampled by [`crate::passes::IblPass`] every frame.
//! Because the bake is deterministic, scene-invariant, and runs
//! exactly once per device session, modelling it as a [`Pass`] would
//! waste a graph slot every frame. Instead the renderer calls
//! [`build_brdf_lut_bake_pipeline`] at startup, dispatches once via
//! its own command encoder, and stores the resulting texture in the
//! [`crate::resources::BrdfLut`] resource slot.
//!
//! PR 7 ships the pipeline-construction surface so the WGSL shader
//! compiles green against a real `engine_gpu::Device`. PR 8 wires
//! the bind-group descriptor + storage-texture output + dispatch +
//! queue submission that actually populates the texture.
//!
//! [`Pass`]: crate::render_graph::Pass

use engine_gpu::{ComputePipeline, Device, RenderPipeline};
use engine_shader::Stage;

use crate::passes;
use crate::shader::{
    ComputePipelineHelperDesc, ShaderError, build_compute_pipeline, wgsl_artefact_set,
};
use crate::shaders::BRDF_LUT_BAKE_WGSL;

/// Build the BRDF LUT bake compute pipeline.
///
/// Consumes the hand-written [`crate::shaders::BRDF_LUT_BAKE_WGSL`]
/// source (Hammersley + GGX importance-sampled BRDF integrator per
/// ADR-065 §3) and assembles a one-shot
/// `engine_gpu::ComputePipeline`. The renderer dispatches this
/// pipeline once at startup; the output is a 512² Rgba16Float
/// [`crate::resources::BrdfLut`] texture sampled by every IBL
/// evaluation thereafter.
pub fn build_brdf_lut_bake_pipeline(device: &Device) -> Result<ComputePipeline, ShaderError> {
    let cs = wgsl_artefact_set(Stage::Compute, "cs_main", BRDF_LUT_BAKE_WGSL);
    build_compute_pipeline(
        device,
        &ComputePipelineHelperDesc {
            label: "init.brdf_lut_bake",
            compute: &cs,
            entry: "cs_main",
        },
    )
}

/// Bundle of every pipeline the Phase-6 Track-A graph constructs.
///
/// Owned by [`build_all_phase6_pipelines`] which returns this struct
/// after exercising the full assembly path. The smoke test asserts
/// every member compiled; the PR-8 runner-validated parity fixtures
/// will reuse the bundle to pre-warm the graph's pass-owned pipelines.
#[derive(Clone, Debug)]
pub struct Phase6Pipelines {
    /// Compute: front-end frustum cull.
    pub cull: ComputePipeline,
    /// Render: 4-cascade CSM depth-only.
    pub csm_shadow: RenderPipeline,
    /// Compute: 16×9×24 cluster-light assignment.
    pub cluster_assign: ComputePipeline,
    /// Render: MRT G-buffer fill.
    pub gbuffer: RenderPipeline,
    /// Compute: SSAO 8-tap Fibonacci.
    pub ssao: ComputePipeline,
    /// Compute: IBL L2-SH evaluation + split-sum.
    pub ibl_evaluate: ComputePipeline,
    /// Render: full-screen Cook-Torrance lighting accumulation.
    pub lighting: RenderPipeline,
    /// Compute: TAA resolve.
    pub taa_resolve: ComputePipeline,
    /// Compute: bloom soft-knee extract.
    pub bloom_extract: ComputePipeline,
    /// Compute: bloom mip-chain downsample.
    pub bloom_downsample: ComputePipeline,
    /// Compute: bloom mip-chain upsample composite.
    pub bloom_upsample: ComputePipeline,
    /// Compute: ACES filmic tonemap.
    pub tonemap: ComputePipeline,
    /// Compute: BRDF LUT bake (one-shot, init-time).
    pub brdf_lut_bake: ComputePipeline,
}

/// Build every Phase-6 GPU pipeline against a freshly-initialised
/// [`Device`]. Smoke-tested as a single end-to-end assembly.
///
/// Returns `Err((pass_name, ShaderError))` on the first failure; the
/// remaining pipelines are not exercised. The named pass labels the
/// failure for the smoke test report and for PR 8's parity-fixture
/// setup.
pub fn build_all_phase6_pipelines(
    device: &Device,
) -> Result<Phase6Pipelines, (&'static str, ShaderError)> {
    Ok(Phase6Pipelines {
        cull: passes::build_cull_pipeline(device).map_err(|e| ("cull", e))?,
        csm_shadow: passes::build_csm_shadow_pipeline(device).map_err(|e| ("shadow", e))?,
        cluster_assign: passes::build_cluster_assign_pipeline(device)
            .map_err(|e| ("light.cluster", e))?,
        gbuffer: passes::build_gbuffer_pipeline(device).map_err(|e| ("draw.opaque", e))?,
        ssao: passes::build_ssao_pipeline(device).map_err(|e| ("post.fx.ssao", e))?,
        ibl_evaluate: passes::build_ibl_evaluate_pipeline(device)
            .map_err(|e| ("draw.opaque.ibl", e))?,
        lighting: passes::build_lighting_pipeline(device).map_err(|e| ("draw.opaque.2", e))?,
        taa_resolve: passes::build_taa_resolve_pipeline(device).map_err(|e| ("post.fx.taa", e))?,
        bloom_extract: passes::build_bloom_extract_pipeline(device)
            .map_err(|e| ("post.fx.bloom.extract", e))?,
        bloom_downsample: passes::build_bloom_downsample_pipeline(device)
            .map_err(|e| ("post.fx.bloom.downsample", e))?,
        bloom_upsample: passes::build_bloom_upsample_pipeline(device)
            .map_err(|e| ("post.fx.bloom.upsample", e))?,
        tonemap: passes::build_tonemap_pipeline(device).map_err(|e| ("post.fx.tonemap", e))?,
        brdf_lut_bake: build_brdf_lut_bake_pipeline(device)
            .map_err(|e| ("init.brdf_lut_bake", e))?,
    })
}
