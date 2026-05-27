// TaaPass — temporal AA resolve (ADR-065 §4).
//
// Workgroup: (8, 8, 1). Reads LitColor + IblContribution + previous
// TaaHistory + GBufferMotionDepth; writes TaaResolvedColor + updates
// TaaHistory ping-pong.
//
// Jitter source MUST match `engine_raster::post_fx::jitter_for_frame`
// — passed in via `TaaUniforms`. The 3×3 neighbourhood clip in YCgCo
// space + the disocclusion mask follow the CPU oracle exactly.
//
// Source-of-truth: `engine_raster::post_fx::taa_resolve`.

struct TaaUniforms {
    prev_view_projection : mat4x4<f32>,
    jitter_current : vec2<f32>,
    jitter_prev : vec2<f32>,
    blend_alpha : f32,
    disocclusion_threshold : f32,
    reserved : vec2<f32>,
};

@group(1) @binding(0) var<uniform> taa : TaaUniforms;
@group(2) @binding(0) var current_color : texture_2d<f32>;
@group(2) @binding(1) var ibl_contribution : texture_2d<f32>;
@group(2) @binding(2) var history : texture_2d<f32>;
@group(2) @binding(3) var motion_depth : texture_2d<f32>;
@group(2) @binding(4) var linear_sampler : sampler;
@group(2) @binding(5) var resolved_out : texture_storage_2d<rgba16float, write>;
@group(2) @binding(6) var history_out : texture_storage_2d<rgba16float, write>;

fn rgb_to_ycgco(c : vec3<f32>) -> vec3<f32> {
    let y = 0.25 * c.r + 0.5 * c.g + 0.25 * c.b;
    let cg = -0.25 * c.r + 0.5 * c.g - 0.25 * c.b;
    let co = 0.5 * c.r - 0.5 * c.b;
    return vec3<f32>(y, cg, co);
}

fn ycgco_to_rgb(c : vec3<f32>) -> vec3<f32> {
    let tmp = c.x - c.y;
    let r = tmp + c.z;
    let g = c.x + c.y;
    let b = tmp - c.z;
    return vec3<f32>(r, g, b);
}

fn fetch(coord : vec2<i32>, dim : vec2<u32>) -> vec3<f32> {
    let clamped = clamp(coord, vec2<i32>(0), vec2<i32>(dim) - vec2<i32>(1));
    return textureLoad(current_color, clamped, 0).rgb
        + textureLoad(ibl_contribution, clamped, 0).rgb;
}

@compute @workgroup_size(8, 8, 1)
fn cs_main(@builtin(global_invocation_id) gid : vec3<u32>) {
    let dim = textureDimensions(resolved_out);
    if (gid.x >= dim.x || gid.y >= dim.y) {
        return;
    }
    let coord = vec2<i32>(gid.xy);

    // Current frame colour + IBL contribution.
    let curr = fetch(coord, dim);

    // 3×3 neighbourhood in YCgCo for AABB clip.
    var ycgco_min = vec3<f32>(1e10);
    var ycgco_max = vec3<f32>(-1e10);
    for (var dy = -1; dy <= 1; dy = dy + 1) {
        for (var dx = -1; dx <= 1; dx = dx + 1) {
            let n = rgb_to_ycgco(fetch(coord + vec2<i32>(dx, dy), dim));
            ycgco_min = min(ycgco_min, n);
            ycgco_max = max(ycgco_max, n);
        }
    }

    // Reproject: motion vector points from current to previous in
    // NDC space. UV delta = motion.xy * 0.5 (NDC → UV scale; y flip
    // is handled by the motion-vector encoding).
    let md = textureLoad(motion_depth, coord, 0);
    let motion = md.xy;
    let uv = (vec2<f32>(coord) + vec2<f32>(0.5)) / vec2<f32>(dim);
    let prev_uv = uv - motion * 0.5;
    let depth = md.z;

    var resolved = curr;
    var write_history = curr;

    if (prev_uv.x >= 0.0 && prev_uv.x <= 1.0 && prev_uv.y >= 0.0 && prev_uv.y <= 1.0) {
        let history_rgb = textureSampleLevel(history, linear_sampler, prev_uv, 0.0).rgb;
        let history_ycgco = rgb_to_ycgco(history_rgb);
        let clipped_ycgco = clamp(history_ycgco, ycgco_min, ycgco_max);
        let clipped_rgb = ycgco_to_rgb(clipped_ycgco);

        // Disocclusion mask: depth-ratio threshold.
        let history_depth = textureSampleLevel(motion_depth, linear_sampler, prev_uv, 0.0).z;
        let depth_ratio = abs(depth - history_depth) / max(max(depth, history_depth), 1e-3);
        let disocclusion = step(taa.disocclusion_threshold, depth_ratio);
        let alpha = mix(taa.blend_alpha, 1.0, disocclusion);

        resolved = mix(clipped_rgb, curr, alpha);
        write_history = resolved;
    }

    textureStore(resolved_out, coord, vec4<f32>(resolved, 1.0));
    textureStore(history_out, coord, vec4<f32>(write_history, 1.0));
}
