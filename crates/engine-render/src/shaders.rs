//! Embedded WGSL shader sources (Phase 6 PR 3.5 + PR 4.5).
//!
//! Each shader corresponds to one Track-A pass. The sources are
//! included at compile time so the engine binary carries them
//! without an asset-pak round-trip — pipelines build directly from
//! these strings via `engine_render::shader::build_render_pipeline`
//! / `build_compute_pipeline`.
//!
//! Why embedded WGSL and not Slang bundles? PR 3.5 / 4.5 ship the
//! shader sources independently of a `slangc` build step so the
//! Phase-6 contract surface lands without the build-environment
//! dependency. A future PR may switch to the bundle-based path (PR 2
//! `ShaderArtefactSet`) when the asset pipeline ships a
//! pre-compiled-shader workflow.
//!
//! All shaders cross-reference the Rust contract types in
//! [`crate::contracts`]; the descriptor layouts named in each
//! shader's WGSL bindings must match those structs' field order +
//! sizes (ADR-064 + ADR-065).

/// CullPass compute shader. Frustum-culls instances, appends
/// surviving entries to the IndirectDrawBuffer.
pub const CULL_WGSL: &str = include_str!("../shaders/cull.wgsl");

/// CsmShadowPass vertex shader. 4-cascade reverse-Z depth-only.
pub const CSM_SHADOW_WGSL: &str = include_str!("../shaders/csm_shadow.wgsl");

/// GBufferPass vertex + fragment. MRT G-buffer fill.
pub const GBUFFER_WGSL: &str = include_str!("../shaders/gbuffer.wgsl");

/// ClusterLightPass compute shader. Per-cell light assignment.
pub const CLUSTER_ASSIGN_WGSL: &str = include_str!("../shaders/cluster_assign.wgsl");

/// LightingAccumulationPass full-screen Cook-Torrance shader.
pub const LIGHTING_WGSL: &str = include_str!("../shaders/lighting.wgsl");

/// SsaoPass 8-tap Fibonacci kernel SSAO compute.
pub const SSAO_WGSL: &str = include_str!("../shaders/ssao.wgsl");

/// BrdfLutBake one-shot 512² LUT bake.
pub const BRDF_LUT_BAKE_WGSL: &str = include_str!("../shaders/brdf_lut_bake.wgsl");

/// IblPass L2 SH evaluation + split-sum specular.
pub const IBL_EVALUATE_WGSL: &str = include_str!("../shaders/ibl_evaluate.wgsl");

/// TaaPass temporal AA resolve.
pub const TAA_RESOLVE_WGSL: &str = include_str!("../shaders/taa_resolve.wgsl");

/// BloomPass soft-knee extract + downsample/upsample chain
/// (three entry points: `cs_extract`, `cs_downsample`, `cs_upsample`).
pub const BLOOM_WGSL: &str = include_str!("../shaders/bloom.wgsl");

/// TonemapPass ACES filmic.
pub const TONEMAP_WGSL: &str = include_str!("../shaders/tonemap.wgsl");

/// UpscalePass owned-bilinear GPU dispatch (Phase 6 PR 1a, ADR-083 §4).
/// `textureSampleLevel` against a linear sampler at the 2× output res.
pub const BILINEAR_UPSCALE_WGSL: &str = include_str!("../shaders/bilinear_upscale.wgsl");

/// UpscalePass FSR-EASU edge-adaptive spatial upsampler (Phase 6 PR 1a,
/// ADR-076 + ADR-083 §3). Polaris-compatible WGSL port of GPUOpen
/// FidelityFX FSR 1.0 (Lottes 2021, MIT licensed).
pub const FSR_EASU_WGSL: &str = include_str!("../shaders/fsr_easu.wgsl");

/// SplatSort radix-by-depth compute kernel (Phase 6 PR 2, ADR-077 §3).
/// Three entry points: `cs_init` projects positions to camera-space
/// depth keys; `cs_count` accumulates the per-pass 8-bit digit bins
/// via atomicAdd; `cs_scatter` performs the stable partition.
pub const SPLAT_SORT_WGSL: &str = include_str!("../shaders/splat_sort.wgsl");

/// SplatComposite back-to-front 2D-Gaussian alpha-blend (Phase 6 PR 2,
/// ADR-077 §4). Vertex stage emits a billboard quad per instance from
/// the sorted permutation; fragment stage evaluates the 2D Gaussian +
/// the 9-coefficient L=2 SH appearance.
pub const SPLAT_COMPOSITE_WGSL: &str = include_str!("../shaders/splat_composite.wgsl");

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_contains(haystack: &str, needle: &str, name: &str) {
        assert!(
            haystack.contains(needle),
            "{name}: WGSL source missing required `{needle}`"
        );
    }

    #[test]
    fn cull_shader_is_a_compute_pipeline() {
        assert_contains(CULL_WGSL, "@compute", "cull.wgsl");
        assert_contains(CULL_WGSL, "@workgroup_size(64", "cull.wgsl");
        assert_contains(CULL_WGSL, "fn cs_main", "cull.wgsl");
    }

    #[test]
    fn csm_shadow_shader_is_vertex_only() {
        assert_contains(CSM_SHADOW_WGSL, "@vertex", "csm_shadow.wgsl");
        assert_contains(CSM_SHADOW_WGSL, "fn vs_main", "csm_shadow.wgsl");
        // Depth-only; no fragment entry point.
        assert!(
            !CSM_SHADOW_WGSL.contains("@fragment"),
            "csm_shadow.wgsl: depth-only pass shouldn't have a fragment shader"
        );
    }

    #[test]
    fn gbuffer_shader_has_vertex_and_fragment() {
        assert_contains(GBUFFER_WGSL, "@vertex", "gbuffer.wgsl");
        assert_contains(GBUFFER_WGSL, "@fragment", "gbuffer.wgsl");
        // Three MRT outputs (albedo_roughness / normal_metallic / motion_depth_id).
        assert_contains(GBUFFER_WGSL, "@location(0)", "gbuffer.wgsl");
        assert_contains(GBUFFER_WGSL, "@location(1)", "gbuffer.wgsl");
        assert_contains(GBUFFER_WGSL, "@location(2)", "gbuffer.wgsl");
    }

    #[test]
    fn cluster_assign_workgroup_matches_contract() {
        assert_contains(CLUSTER_ASSIGN_WGSL, "@compute", "cluster_assign.wgsl");
        // Matches contracts::CLUSTER_ASSIGN_WORKGROUP_SIZE = [16, 9, 1].
        assert_contains(
            CLUSTER_ASSIGN_WGSL,
            "@workgroup_size(16, 9, 1)",
            "cluster_assign.wgsl",
        );
        // Matches contracts::MAX_LIGHTS_PER_CLUSTER = 32.
        assert_contains(
            CLUSTER_ASSIGN_WGSL,
            "MAX_LIGHTS_PER_CLUSTER : u32 = 32u",
            "cluster_assign.wgsl",
        );
    }

    #[test]
    fn lighting_shader_has_full_screen_triangle() {
        assert_contains(LIGHTING_WGSL, "@vertex", "lighting.wgsl");
        assert_contains(LIGHTING_WGSL, "@fragment", "lighting.wgsl");
        // Full-screen triangle uses 3 vertices, sourced from the
        // vertex-index builtin (no vertex buffer).
        assert_contains(LIGHTING_WGSL, "vertex_index", "lighting.wgsl");
    }

    #[test]
    fn lighting_shader_references_cook_torrance() {
        // The Cook-Torrance GGX/Smith-Schlick BRDF is the contract;
        // an audit must be able to see that the shader implements it.
        assert_contains(LIGHTING_WGSL, "ggx_d", "lighting.wgsl");
        assert_contains(LIGHTING_WGSL, "smith_g", "lighting.wgsl");
        assert_contains(LIGHTING_WGSL, "schlick_f", "lighting.wgsl");
        assert_contains(LIGHTING_WGSL, "cook_torrance", "lighting.wgsl");
    }

    #[test]
    fn ssao_shader_uses_fibonacci_kernel() {
        assert_contains(SSAO_WGSL, "@compute", "ssao.wgsl");
        assert_contains(SSAO_WGSL, "SSAO_KERNEL_TAPS : u32 = 8u", "ssao.wgsl");
        assert_contains(SSAO_WGSL, "fn cs_main", "ssao.wgsl");
    }

    #[test]
    fn brdf_lut_bake_uses_hammersley_importance_sample() {
        assert_contains(BRDF_LUT_BAKE_WGSL, "@compute", "brdf_lut_bake.wgsl");
        assert_contains(BRDF_LUT_BAKE_WGSL, "hammersley", "brdf_lut_bake.wgsl");
        assert_contains(
            BRDF_LUT_BAKE_WGSL,
            "ggx_importance_sample",
            "brdf_lut_bake.wgsl",
        );
        assert_contains(BRDF_LUT_BAKE_WGSL, "integrate_brdf", "brdf_lut_bake.wgsl");
    }

    #[test]
    fn ibl_shader_evaluates_l2_sh() {
        assert_contains(IBL_EVALUATE_WGSL, "@compute", "ibl_evaluate.wgsl");
        assert_contains(IBL_EVALUATE_WGSL, "evaluate_sh", "ibl_evaluate.wgsl");
        assert_contains(IBL_EVALUATE_WGSL, "sh_coeffs", "ibl_evaluate.wgsl");
    }

    #[test]
    fn taa_shader_clips_in_ycgco_with_disocclusion() {
        assert_contains(TAA_RESOLVE_WGSL, "@compute", "taa_resolve.wgsl");
        assert_contains(TAA_RESOLVE_WGSL, "rgb_to_ycgco", "taa_resolve.wgsl");
        assert_contains(TAA_RESOLVE_WGSL, "ycgco_to_rgb", "taa_resolve.wgsl");
        assert_contains(
            TAA_RESOLVE_WGSL,
            "disocclusion_threshold",
            "taa_resolve.wgsl",
        );
    }

    #[test]
    fn bloom_shader_has_three_entry_points() {
        assert_contains(BLOOM_WGSL, "fn cs_extract", "bloom.wgsl");
        assert_contains(BLOOM_WGSL, "fn cs_downsample", "bloom.wgsl");
        assert_contains(BLOOM_WGSL, "fn cs_upsample", "bloom.wgsl");
    }

    #[test]
    fn bilinear_upscale_shader_has_cs_main() {
        assert_contains(BILINEAR_UPSCALE_WGSL, "@compute", "bilinear_upscale.wgsl");
        assert_contains(
            BILINEAR_UPSCALE_WGSL,
            "@workgroup_size(8, 8, 1)",
            "bilinear_upscale.wgsl",
        );
        assert_contains(BILINEAR_UPSCALE_WGSL, "fn cs_main", "bilinear_upscale.wgsl");
        assert_contains(
            BILINEAR_UPSCALE_WGSL,
            "textureSampleLevel",
            "bilinear_upscale.wgsl",
        );
    }

    #[test]
    fn fsr_easu_shader_is_polaris_compatible() {
        // ADR-076 + ADR-083: Polaris GFX8 compatibility — no subgroup
        // intrinsics, no f16. EASU runs on every device the engine
        // targets. The check scans for actual intrinsic call sites
        // rather than the literal word so documentation comments can
        // describe the constraint.
        assert_contains(FSR_EASU_WGSL, "@compute", "fsr_easu.wgsl");
        assert_contains(FSR_EASU_WGSL, "@workgroup_size(8, 8, 1)", "fsr_easu.wgsl");
        assert_contains(FSR_EASU_WGSL, "fn cs_main", "fsr_easu.wgsl");
        assert_contains(FSR_EASU_WGSL, "luma_rec709", "fsr_easu.wgsl");
        assert_contains(FSR_EASU_WGSL, "easu_set", "fsr_easu.wgsl");
        assert!(
            !FSR_EASU_WGSL.contains("subgroupShuffle")
                && !FSR_EASU_WGSL.contains("subgroupBallot")
                && !FSR_EASU_WGSL.contains("subgroupBroadcast"),
            "fsr_easu.wgsl: Polaris-compatibility requires no subgroup intrinsics"
        );
        // The `f16` half-precision type does not appear (it would
        // require the `f16` WGSL extension which Polaris lacks).
        assert!(
            !FSR_EASU_WGSL.contains(": f16")
                && !FSR_EASU_WGSL.contains("vec2<f16>")
                && !FSR_EASU_WGSL.contains("vec3<f16>")
                && !FSR_EASU_WGSL.contains("vec4<f16>"),
            "fsr_easu.wgsl: Polaris-compatibility requires pure f32"
        );
    }

    #[test]
    fn splat_sort_shader_has_three_radix_entry_points() {
        assert_contains(SPLAT_SORT_WGSL, "@compute", "splat_sort.wgsl");
        assert_contains(SPLAT_SORT_WGSL, "fn cs_init", "splat_sort.wgsl");
        assert_contains(SPLAT_SORT_WGSL, "fn cs_count", "splat_sort.wgsl");
        assert_contains(SPLAT_SORT_WGSL, "fn cs_scatter", "splat_sort.wgsl");
        // Polaris-compat: no subgroup intrinsics, no f16 types.
        assert!(
            !SPLAT_SORT_WGSL.contains("subgroupShuffle")
                && !SPLAT_SORT_WGSL.contains("subgroupBallot")
                && !SPLAT_SORT_WGSL.contains("subgroupBroadcast"),
            "splat_sort.wgsl: no subgroup intrinsics permitted"
        );
        assert!(
            !SPLAT_SORT_WGSL.contains(": f16")
                && !SPLAT_SORT_WGSL.contains("vec2<f16>")
                && !SPLAT_SORT_WGSL.contains("vec3<f16>")
                && !SPLAT_SORT_WGSL.contains("vec4<f16>"),
            "splat_sort.wgsl: pure f32 only"
        );
    }

    #[test]
    fn splat_composite_shader_has_billboard_quad_and_sh() {
        assert_contains(SPLAT_COMPOSITE_WGSL, "@vertex", "splat_composite.wgsl");
        assert_contains(SPLAT_COMPOSITE_WGSL, "@fragment", "splat_composite.wgsl");
        assert_contains(SPLAT_COMPOSITE_WGSL, "evaluate_sh", "splat_composite.wgsl");
        assert_contains(SPLAT_COMPOSITE_WGSL, "QUAD_VERTS", "splat_composite.wgsl");
    }

    #[test]
    fn tonemap_shader_implements_aces_filmic() {
        // Phase 5.5 A.3 Slice 8: aligned to the Narkowicz 2015 ACES fit
        // (`engine_raster::post_fx::tonemap_aces`); the prior Hill 2017
        // RRT/ODT fit diverged from the CPU oracle by a constant
        // `white_point` divisor and was the largest contributor to
        // pre-Slice-8 parity gap.
        assert_contains(TONEMAP_WGSL, "@compute", "tonemap.wgsl");
        assert_contains(TONEMAP_WGSL, "aces_narkowicz_component", "tonemap.wgsl");
        assert_contains(TONEMAP_WGSL, "aces_filmic", "tonemap.wgsl");
    }
}
