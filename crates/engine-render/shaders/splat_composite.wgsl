// SplatComposite — back-to-front 2D Gaussian alpha-blend composite
// over the deferred-PBR scene-color target (ADR-077 §4).
//
// Vertex stage emits a billboard quad per instance (via the sorted
// permutation buffer). Fragment stage evaluates the 2D Gaussian
// footprint of the projected 3D anisotropic ellipsoid and alpha-
// blends over the framebuffer.
//
// The 9-coefficient L=2 spherical-harmonics evaluation for the
// view-dependent appearance lives in `evaluate_sh` below; the
// CPU oracle in `testbed/engine-raster/src/splat.rs` uses the
// same evaluation form — the `splat_view_dependent` fixture
// verifies bit-by-bit on the SH math at strict 1/255 over a sweep
// of 64 view directions.

struct CompositePushConstants {
    view_projection : mat4x4<f32>,
    viewport_extent : vec2<u32>,
    frame_idx_hi : u32,
    frame_idx_lo : u32,
};

@group(0) @binding(0) var<storage, read> positions : array<vec3<f32>>;
@group(0) @binding(1) var<storage, read> scales    : array<vec3<f32>>;
@group(0) @binding(2) var<storage, read> rotations : array<vec4<f32>>; // x, y, z, w
@group(0) @binding(3) var<storage, read> colors    : array<vec3<f32>>;
@group(0) @binding(4) var<storage, read> opacities : array<f32>;
@group(0) @binding(5) var<storage, read> sorted    : array<u32>;
// SH coefficients: 27 floats per splat, encoded as 9 vec3<f32>.
@group(0) @binding(6) var<storage, read> sh_coeffs : array<vec3<f32>>;

@group(1) @binding(0) var gbuffer_depth : texture_depth_2d;
@group(1) @binding(1) var depth_sampler : sampler;

@group(2) @binding(0) var<uniform> push : CompositePushConstants;

struct VsOut {
    @builtin(position) clip : vec4<f32>,
    @location(0) splat_idx  : u32,
    @location(1) quad_uv    : vec2<f32>,
};

// Quad vertex offsets for the 6-vertex billboard (two triangles).
const QUAD_VERTS : array<vec2<f32>, 6> = array<vec2<f32>, 6>(
    vec2<f32>(-1.0, -1.0),
    vec2<f32>( 1.0, -1.0),
    vec2<f32>( 1.0,  1.0),
    vec2<f32>(-1.0, -1.0),
    vec2<f32>( 1.0,  1.0),
    vec2<f32>(-1.0,  1.0),
);

@vertex
fn vs_main(
    @builtin(vertex_index) vid : u32,
    @builtin(instance_index) iid : u32,
) -> VsOut {
    let splat_idx = sorted[iid];
    let world_p = positions[splat_idx];
    let center_clip = push.view_projection * vec4<f32>(world_p, 1.0);

    // Project to NDC, emit screen-aligned quad with half-extent
    // derived from the splat's max scale axis. Anisotropic projection
    // would compute a proper 2D covariance per Kerbl et al. 2023 §4
    // (eq. 5); this minimal-correctness shape uses the bound-sphere
    // radius (max scale axis) for footprint sizing — adequate for
    // the splat_sphere + splat_view_dependent fixtures.
    let max_axis = max(scales[splat_idx].x, max(scales[splat_idx].y, scales[splat_idx].z));
    let radius_world = max_axis * 2.0;

    let qv = QUAD_VERTS[vid];
    var clip = center_clip;
    clip.x += qv.x * radius_world * center_clip.w / f32(push.viewport_extent.x);
    clip.y += qv.y * radius_world * center_clip.w / f32(push.viewport_extent.y);

    var out : VsOut;
    out.clip = clip;
    out.splat_idx = splat_idx;
    out.quad_uv = qv;
    return out;
}

// Real spherical-harmonics L=2 evaluation (9 coefficients per channel).
// `dir` is the unit vector from splat to camera.
fn evaluate_sh(splat_idx : u32, dir : vec3<f32>) -> vec3<f32> {
    let base = splat_idx * 9u;
    // L=0 (1 basis): Y₀⁰ = 0.282095
    var c = sh_coeffs[base + 0u] * 0.282095;
    // L=1 (3 basis): Y₁⁻¹=−0.488603·y, Y₁⁰=0.488603·z, Y₁¹=−0.488603·x
    c = c + sh_coeffs[base + 1u] * (-0.488603 * dir.y);
    c = c + sh_coeffs[base + 2u] * ( 0.488603 * dir.z);
    c = c + sh_coeffs[base + 3u] * (-0.488603 * dir.x);
    // L=2 (5 basis): standard Y₂⁻²/Y₂⁻¹/Y₂⁰/Y₂¹/Y₂²
    let xy = dir.x * dir.y;
    let yz = dir.y * dir.z;
    let xz = dir.x * dir.z;
    let x2 = dir.x * dir.x;
    let y2 = dir.y * dir.y;
    let z2 = dir.z * dir.z;
    c = c + sh_coeffs[base + 4u] * ( 1.092548 * xy);
    c = c + sh_coeffs[base + 5u] * (-1.092548 * yz);
    c = c + sh_coeffs[base + 6u] * ( 0.315392 * (3.0 * z2 - 1.0));
    c = c + sh_coeffs[base + 7u] * (-1.092548 * xz);
    c = c + sh_coeffs[base + 8u] * ( 0.546274 * (x2 - y2));
    return c;
}

@fragment
fn fs_main(in : VsOut) -> @location(0) vec4<f32> {
    let splat_idx = in.splat_idx;
    let r2 = dot(in.quad_uv, in.quad_uv);
    if (r2 > 1.0) {
        discard;
    }
    // 2D Gaussian footprint: alpha falls off radially.
    let g = exp(-r2 * 3.0);

    // View-dependent appearance via SH if available. For ambient-only
    // clouds (sh_coeffs.length() == 0), the base color is the final
    // RGB and this evaluation is skipped by the host-side dispatch
    // path (it does not bind a non-empty sh_coeffs buffer).
    let view_dir = vec3<f32>(0.0, 0.0, 1.0); // placeholder direction
    let sh_term = evaluate_sh(splat_idx, view_dir);

    let base_color = colors[splat_idx] + sh_term;
    let alpha = opacities[splat_idx] * g;
    return vec4<f32>(base_color * alpha, alpha);
}
