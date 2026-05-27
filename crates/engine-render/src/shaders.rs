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
}
