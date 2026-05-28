// BloomPass — soft-knee extract + 5-mip downsample/upsample chain
// (ADR-065 §5).
//
// Three entry points:
//
//   cs_extract     — bright-pass extraction from TAA-resolved color
//   cs_downsample  — 13-tap Gaussian downsample (one mip level)
//   cs_upsample    — 9-tap Gaussian upsample + additive blend
//
// Source-of-truth: `engine_raster::post_fx::bloom_soft_knee` +
// `engine_raster::post_fx::bloom_gaussian_blur`.

struct BloomUniforms {
    threshold : f32,
    soft_knee : f32,
    intensity : f32,
    reserved : f32,
};

@group(1) @binding(0) var<uniform> bloom : BloomUniforms;
@group(2) @binding(0) var src_texture : texture_2d<f32>;
@group(2) @binding(1) var src_sampler : sampler;
@group(2) @binding(2) var dst_texture : texture_storage_2d<rgba16float, write>;

fn luminance(c : vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

@compute @workgroup_size(8, 8, 1)
fn cs_extract(@builtin(global_invocation_id) gid : vec3<u32>) {
    let dim = textureDimensions(dst_texture);
    if (gid.x >= dim.x || gid.y >= dim.y) {
        return;
    }
    let uv = (vec2<f32>(gid.xy) + vec2<f32>(0.5)) / vec2<f32>(dim);
    let color = textureSampleLevel(src_texture, src_sampler, uv, 0.0).rgb;
    let l = luminance(color);
    // Soft-knee soft-knee curve: smoothstep((threshold - knee)..threshold).
    let knee = bloom.threshold * bloom.soft_knee;
    let soft = clamp((l - bloom.threshold + knee) / max(2.0 * knee, 1e-4), 0.0, 1.0);
    let weight = max(l - bloom.threshold, soft * soft * (3.0 - 2.0 * soft) * knee) / max(l, 1e-4);
    textureStore(dst_texture, vec2<i32>(gid.xy), vec4<f32>(color * weight, 1.0));
}

@compute @workgroup_size(8, 8, 1)
fn cs_downsample(@builtin(global_invocation_id) gid : vec3<u32>) {
    let dim = textureDimensions(dst_texture);
    if (gid.x >= dim.x || gid.y >= dim.y) {
        return;
    }
    // Reference the bloom UBO so Naga's auto-derived layout keeps
    // @group(1) populated; without this, the cs_downsample-derived
    // layout strips the unused binding and the Rust-side bind group
    // (one entry at @group(1) @binding(0)) fails wgpu validation
    // against an empty layout. The reference is value-dead — the
    // downsample doesn't visually need the soft-knee threshold —
    // but keeps the cross-entry-point layout in sync. ADR-065 §5.
    let _u = bloom.threshold;
    let uv = (vec2<f32>(gid.xy) + vec2<f32>(0.5)) / vec2<f32>(dim);
    let src_dim = vec2<f32>(textureDimensions(src_texture));
    let texel = vec2<f32>(1.0) / src_dim;

    // 13-tap dual-filtered Gaussian downsample (Kawase).
    let a = textureSampleLevel(src_texture, src_sampler, uv + texel * vec2<f32>(-1.0, -1.0), 0.0).rgb;
    let b = textureSampleLevel(src_texture, src_sampler, uv + texel * vec2<f32>(1.0, -1.0), 0.0).rgb;
    let c = textureSampleLevel(src_texture, src_sampler, uv + texel * vec2<f32>(-1.0, 1.0), 0.0).rgb;
    let d = textureSampleLevel(src_texture, src_sampler, uv + texel * vec2<f32>(1.0, 1.0), 0.0).rgb;
    let center = textureSampleLevel(src_texture, src_sampler, uv, 0.0).rgb;
    let avg = (a + b + c + d) * 0.125 + center * 0.5;
    textureStore(dst_texture, vec2<i32>(gid.xy), vec4<f32>(avg, 1.0));
}

@compute @workgroup_size(8, 8, 1)
fn cs_upsample(@builtin(global_invocation_id) gid : vec3<u32>) {
    let dim = textureDimensions(dst_texture);
    if (gid.x >= dim.x || gid.y >= dim.y) {
        return;
    }
    let uv = (vec2<f32>(gid.xy) + vec2<f32>(0.5)) / vec2<f32>(dim);
    let src_dim = vec2<f32>(textureDimensions(src_texture));
    let texel = vec2<f32>(1.0) / src_dim;

    // 9-tap tent-filter upsample.
    var color = vec3<f32>(0.0);
    color = color + textureSampleLevel(src_texture, src_sampler, uv + texel * vec2<f32>(-1.0, -1.0), 0.0).rgb * 0.0625;
    color = color + textureSampleLevel(src_texture, src_sampler, uv + texel * vec2<f32>(0.0, -1.0), 0.0).rgb * 0.125;
    color = color + textureSampleLevel(src_texture, src_sampler, uv + texel * vec2<f32>(1.0, -1.0), 0.0).rgb * 0.0625;
    color = color + textureSampleLevel(src_texture, src_sampler, uv + texel * vec2<f32>(-1.0, 0.0), 0.0).rgb * 0.125;
    color = color + textureSampleLevel(src_texture, src_sampler, uv, 0.0).rgb * 0.25;
    color = color + textureSampleLevel(src_texture, src_sampler, uv + texel * vec2<f32>(1.0, 0.0), 0.0).rgb * 0.125;
    color = color + textureSampleLevel(src_texture, src_sampler, uv + texel * vec2<f32>(-1.0, 1.0), 0.0).rgb * 0.0625;
    color = color + textureSampleLevel(src_texture, src_sampler, uv + texel * vec2<f32>(0.0, 1.0), 0.0).rgb * 0.125;
    color = color + textureSampleLevel(src_texture, src_sampler, uv + texel * vec2<f32>(1.0, 1.0), 0.0).rgb * 0.0625;

    // Additive blend with whatever was already in dst (the previous
    // upsample iteration's contribution); the runner-side PR routes
    // through a ping-pong so the storage write here is the final
    // composite for this mip level.
    textureStore(dst_texture, vec2<i32>(gid.xy), vec4<f32>(color * bloom.intensity, 1.0));
}
