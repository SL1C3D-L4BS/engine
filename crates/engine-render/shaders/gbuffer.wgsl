// GBufferPass — MRT G-buffer fill (ADR-064 §3).
//
// Outputs:
//   slot 0  Bgra8UnormSrgb  RGB albedo (sRGB) + A roughness (linear)
//   slot 1  Rgba16Float     RGB normal (world-space) + A metallic
//   slot 2  Rgba16Float     RG motion vector + B view depth + A id
//   depth   Depth32Float    reverse-Z
//
// Group 0: per-frame uniforms (view + projection + jitter).
// Group 1: per-instance SSBO indexed by `@builtin(instance_index)`
//   (Phase 5.5 A.2d.b — replaces the pre-A.2d `var<immediate> push`
//   declaration so a single `multi_draw_indexed_indirect_count` can
//   consume the CullPass's per-survivor draw-arg array).
// Group 2: per-material textures via bindless heap (ADR-044) — read
//   by `material_index` from the SSBO entry.

struct InstanceDraw {
    model_xform_0 : vec4<f32>,
    model_xform_1 : vec4<f32>,
    model_xform_2 : vec4<f32>,
    material_index : u32,
    instance_id : u32,
    flags : u32,
    reserved : u32,
};

struct PerFrame {
    view_projection : mat4x4<f32>,
    prev_view_projection : mat4x4<f32>,
    view : mat4x4<f32>,
    jitter : vec4<f32>,      // .xy = current, .zw = previous
    camera_pos : vec4<f32>,
};

@group(0) @binding(0) var<uniform> frame : PerFrame;
@group(1) @binding(0) var<storage, read> instances : array<InstanceDraw>;

struct VertexInput {
    @location(0) position : vec3<f32>,
    @location(1) normal : vec3<f32>,
    @location(2) tangent : vec4<f32>,
    @location(3) uv0 : vec2<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position : vec4<f32>,
    @location(0) world_position : vec3<f32>,
    @location(1) world_normal : vec3<f32>,
    @location(2) uv : vec2<f32>,
    @location(3) view_depth : f32,
    @location(4) prev_clip : vec4<f32>,
    @location(5) @interpolate(flat) material_index : u32,
    @location(6) @interpolate(flat) instance_id : u32,
};

fn unpack_model(p : InstanceDraw) -> mat4x4<f32> {
    return mat4x4<f32>(
        vec4<f32>(p.model_xform_0.xyz, 0.0),
        vec4<f32>(p.model_xform_1.xyz, 0.0),
        vec4<f32>(p.model_xform_2.xyz, 0.0),
        vec4<f32>(p.model_xform_0.w, p.model_xform_1.w, p.model_xform_2.w, 1.0),
    );
}

@vertex
fn vs_main(input : VertexInput, @builtin(instance_index) iid : u32) -> VertexOutput {
    let inst = instances[iid];
    let m = unpack_model(inst);
    let world = m * vec4<f32>(input.position, 1.0);
    let world_n = normalize((m * vec4<f32>(input.normal, 0.0)).xyz);
    let view_pos = frame.view * world;
    let clip = frame.view_projection * world;
    let prev = frame.prev_view_projection * world;

    var out : VertexOutput;
    // Apply current-frame jitter directly in NDC.
    let jittered = vec4<f32>(clip.xy + frame.jitter.xy * clip.w, clip.zw);
    out.clip_position = jittered;
    out.world_position = world.xyz;
    out.world_normal = world_n;
    out.uv = input.uv0;
    out.view_depth = -view_pos.z;
    out.prev_clip = prev;
    out.material_index = inst.material_index;
    out.instance_id = inst.instance_id;
    return out;
}

struct GBufferOutput {
    @location(0) albedo_roughness : vec4<f32>,
    @location(1) normal_metallic : vec4<f32>,
    @location(2) motion_depth_id : vec4<f32>,
};

@fragment
fn fs_main(in : VertexOutput) -> GBufferOutput {
    // Phase 6 PR 3.5 ships a minimal material evaluation: each draw
    // contributes its (material_index-derived) tint as a placeholder
    // for the bindless texture sample that the runner-validated PR
    // will wire. The contract — emit into the three MRT slots in the
    // documented format — is what's load-bearing here.
    let mat_color = vec3<f32>(
        f32((in.material_index >> 0u) & 0xffu) / 255.0,
        f32((in.material_index >> 8u) & 0xffu) / 255.0,
        f32((in.material_index >> 16u) & 0xffu) / 255.0,
    );
    let roughness = 0.5;
    let metallic = 0.0;

    // Motion vector: NDC delta between current and previous frame.
    let prev_ndc = in.prev_clip.xy / max(in.prev_clip.w, 1e-6);
    let curr_ndc = in.clip_position.xy / max(in.clip_position.w, 1e-6);
    let motion = curr_ndc - prev_ndc;

    var out : GBufferOutput;
    out.albedo_roughness = vec4<f32>(mat_color, roughness);
    out.normal_metallic = vec4<f32>(normalize(in.world_normal), metallic);
    out.motion_depth_id = vec4<f32>(motion, in.view_depth, f32(in.instance_id));
    return out;
}
