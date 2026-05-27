// CsmShadowPass — 4-cascade depth-only shadow draws (ADR-040 + ADR-064 §4).
//
// Vertex-only pipeline (no fragment shader); the depth attachment is
// the cascade's quadrant of the 4096² D32F atlas. Reverse-Z
// convention (depth = 1.0 at near, 0.0 at far) — comparator must be
// `Greater`.
//
// One draw per cascade; the application binds the cascade index in
// push-constants + the cascade view-projection in Group 1 uniforms.
// The atlas viewport is set per-cascade so all four cascades share
// one texture handle.

struct PushConstants {
    model_xform_0 : vec4<f32>,    // 3x4 model affine, row 0
    model_xform_1 : vec4<f32>,    // row 1
    model_xform_2 : vec4<f32>,    // row 2
    material_index : u32,
    instance_id : u32,
    flags : u32,
    reserved : u32,
};

struct CsmUniforms {
    cascade_vp : array<mat4x4<f32>, 4>,
    cascade_split_far : array<vec4<f32>, 1>, // packed 4 floats
    filter_radius_px : f32,
    bias_constant : f32,
    bias_slope : f32,
};

var<push_constant> push : PushConstants;
@group(1) @binding(0) var<uniform> csm : CsmUniforms;

struct VertexInput {
    @location(0) position : vec3<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position : vec4<f32>,
};

@vertex
fn vs_main(input : VertexInput, @builtin(view_index) cascade : u32) -> VertexOutput {
    let m = mat4x4<f32>(
        vec4<f32>(push.model_xform_0.xyz, 0.0),
        vec4<f32>(push.model_xform_1.xyz, 0.0),
        vec4<f32>(push.model_xform_2.xyz, 0.0),
        vec4<f32>(push.model_xform_0.w, push.model_xform_1.w, push.model_xform_2.w, 1.0),
    );
    let world = m * vec4<f32>(input.position, 1.0);
    let cascade_idx = clamp(cascade, 0u, 3u);
    let vp = csm.cascade_vp[cascade_idx];
    var out : VertexOutput;
    out.clip_position = vp * world;
    return out;
}
