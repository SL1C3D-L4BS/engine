// CsmShadowPass — 4-cascade depth-only shadow draws (ADR-040 + ADR-064 §4).
//
// Vertex-only pipeline (no fragment shader); the depth attachment is
// the cascade's quadrant of the 4096² D32F atlas. Reverse-Z
// convention (depth = 1.0 at near, 0.0 at far) — comparator must be
// `Greater`.
//
// Phase 5.5 A.2d.b: per-draw payload moved from `var<immediate> push`
// (push-constants) to a per-instance SSBO indexed by
// `@builtin(instance_index)`. The CullPass writes
// `DrawIndirect.first_instance = entry.instance_id`; the GPU-side
// `instance_index` builtin then equals `instance_id` (when
// `INDIRECT_FIRST_INSTANCE` is enabled — ADR-074 §5). One
// `multi_draw_indexed_indirect_count` consumes the cull pass's
// draw-arg array + draw-count atomic in a single call.

struct InstanceDraw {
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

@group(0) @binding(0) var<storage, read> instances : array<InstanceDraw>;
@group(1) @binding(0) var<uniform> csm : CsmUniforms;

struct VertexInput {
    @location(0) position : vec3<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position : vec4<f32>,
};

@vertex
fn vs_main(
    input : VertexInput,
    @builtin(instance_index) iid : u32,
    @builtin(view_index) cascade : u32,
) -> VertexOutput {
    let inst = instances[iid];
    let m = mat4x4<f32>(
        vec4<f32>(inst.model_xform_0.xyz, 0.0),
        vec4<f32>(inst.model_xform_1.xyz, 0.0),
        vec4<f32>(inst.model_xform_2.xyz, 0.0),
        vec4<f32>(inst.model_xform_0.w, inst.model_xform_1.w, inst.model_xform_2.w, 1.0),
    );
    let world = m * vec4<f32>(input.position, 1.0);
    let cascade_idx = clamp(cascade, 0u, 3u);
    let vp = csm.cascade_vp[cascade_idx];
    var out : VertexOutput;
    out.clip_position = vp * world;
    return out;
}
