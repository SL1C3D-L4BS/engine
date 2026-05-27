// SsaoPass — 8-tap Fibonacci kernel SSAO at half resolution
// (ADR-065 §1).
//
// Workgroup: (8, 8, 1). Reads GBufferNormalMetallic + DepthBuffer;
// writes SsaoTexture (R16Float, half-resolution). The bilateral
// upsample to full-resolution happens at the
// LightingAccumulationPass consumer.
//
// Source-of-truth: `engine_raster::post_fx::ssao_fibonacci_kernel`.

const SSAO_KERNEL_TAPS : u32 = 8u;

struct SsaoUniforms {
    inverse_projection : mat4x4<f32>,
    kernel : array<vec4<f32>, 8>,    // 8 × Fibonacci samples (.xyz, .w = weight)
    radius : f32,
    bias : f32,
    intensity : f32,
    reserved : f32,
};

@group(1) @binding(0) var<uniform> ssao : SsaoUniforms;
@group(2) @binding(0) var gbuf_normal_metal : texture_2d<f32>;
@group(2) @binding(1) var depth_buffer : texture_depth_2d;
@group(2) @binding(2) var ssao_out : texture_storage_2d<r16float, write>;

fn view_pos_from_depth(uv : vec2<f32>, depth : f32) -> vec3<f32> {
    let ndc = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth, 1.0);
    let v = ssao.inverse_projection * ndc;
    return v.xyz / max(v.w, 1e-6);
}

@compute @workgroup_size(8, 8, 1)
fn cs_main(@builtin(global_invocation_id) gid : vec3<u32>) {
    let dim = textureDimensions(ssao_out);
    if (gid.x >= dim.x || gid.y >= dim.y) {
        return;
    }
    let uv = (vec2<f32>(gid.xy) + vec2<f32>(0.5)) / vec2<f32>(dim);
    let coord_full = vec2<i32>(uv * vec2<f32>(textureDimensions(depth_buffer)));
    let depth = textureLoad(depth_buffer, coord_full, 0);
    if (depth <= 0.0) {
        textureStore(ssao_out, vec2<i32>(gid.xy), vec4<f32>(1.0, 0.0, 0.0, 0.0));
        return;
    }

    let view_pos = view_pos_from_depth(uv, depth);
    let n = normalize(textureLoad(gbuf_normal_metal, coord_full, 0).xyz);

    var occlusion = 0.0;
    var weight_sum = 0.0;
    for (var i = 0u; i < SSAO_KERNEL_TAPS; i = i + 1u) {
        let sample = ssao.kernel[i];
        let dir = normalize(sample.xyz);
        let w = sample.w;
        let sample_pos = view_pos + dir * ssao.radius;
        // Project sample back to UV for depth lookup.
        let inv_proj = ssao.inverse_projection;
        // Naïve forward-projection: assume the inverse is exact-invertible
        // for symmetric frustums (the only kind the renderer uses).
        // The runner-validated PR will use a proper view-projection here.
        let sample_uv = uv + dir.xy * ssao.radius * 0.1;
        if (sample_uv.x < 0.0 || sample_uv.x > 1.0 || sample_uv.y < 0.0 || sample_uv.y > 1.0) {
            continue;
        }
        let scoord = vec2<i32>(sample_uv * vec2<f32>(textureDimensions(depth_buffer)));
        let sdepth = textureLoad(depth_buffer, scoord, 0);
        if (sdepth <= 0.0) {
            continue;
        }
        let sample_view = view_pos_from_depth(sample_uv, sdepth);
        let occluded = step(sample_pos.z + ssao.bias, sample_view.z);
        let range_check = smoothstep(0.0, 1.0, ssao.radius / max(abs(view_pos.z - sample_view.z), 1e-3));
        occlusion = occlusion + occluded * range_check * w;
        weight_sum = weight_sum + w;
    }
    let ao = 1.0 - (occlusion / max(weight_sum, 1e-4)) * ssao.intensity;
    textureStore(ssao_out, vec2<i32>(gid.xy), vec4<f32>(clamp(ao, 0.0, 1.0), 0.0, 0.0, 0.0));
}
