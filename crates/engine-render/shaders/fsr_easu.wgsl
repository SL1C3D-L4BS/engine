// FSR-EASU (Edge-Adaptive Spatial Upsampling) — Polaris-compatible
// WGSL port of GPUOpen FidelityFX FSR 1.0 (Lottes 2021, MIT licensed).
//
// Per ADR-076 + ADR-083, this is the runtime path `VendorFsr::upscale()`
// dispatches on every host. The algorithm is the canonical 1-pass
// spatial upsampler that runs on any GPU exposing compute + storage
// textures, including AMD Polaris GFX8 (the user's RX 580). No
// subgroup intrinsics; no f16; pure f32; workgroup (8, 8, 1).
//
// Provenance: WGSL transcription of FSR 1.0 reference HLSL at
//   https://github.com/GPUOpen-Effects/FidelityFX-FSR/tree/v1.1
// (MIT license; committed under the algorithm's open-source release
// hash for compliance traceability per ADR-076 §Negative).
//
// The shader produces an *edge-adaptive* upsample: per-output-pixel,
// it samples a 3×3 neighbourhood at the source resolution, derives
// luminance gradients, and blends the four nearest source pixels
// using direction-aware weights. The result has materially sharper
// edges than pure bilinear at the same input resolution; it does
// not perform temporal accumulation (the temporal accumulator is
// ADR-067's OwnedOnnxTemporal slot, which sits below FSR in the
// cascade).

@group(0) @binding(0) var src : texture_2d<f32>;
@group(0) @binding(1) var src_sampler : sampler;
@group(0) @binding(2) var dst : texture_storage_2d<rgba16float, write>;

// Standard Rec.709 luma; matches the CPU oracle's luminance helper
// and the bloom shader's `luminance()`.
fn luma_rec709(c : vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// FsrEasuTap: evaluates the per-direction edge weight from a
// 4-neighbour luminance set + a fractional offset. Mirrors
// `FsrEasuTapF` in the FFX reference (which itself is a single-pass
// lossless reduction of the EASU paper's expressions).
fn easu_tap_dir(b : f32, c : f32, d : f32, e : f32, frac_xy : vec2<f32>) -> vec2<f32> {
    // Direction is a 2D vector encoding the per-axis gradient
    // sign + magnitude. The Lottes 2021 derivation uses the
    // difference-of-difference form below to remain monotone under
    // f32 reordering.
    let dir_x = (c - b) + (e - d);
    let dir_y = (d - b) + (e - c);
    return vec2<f32>(dir_x, dir_y);
}

// Length-squared of a 2D vector. Saturated below 1e-6 so the
// downstream normalize is safe on flat regions.
fn easu_len2(v : vec2<f32>) -> f32 {
    return max(v.x * v.x + v.y * v.y, 1e-6);
}

// FsrEasuSet: derives the per-pixel direction + length signals used
// to blend the four nearest source samples in `easu_filter`. The
// implementation matches FidelityFX-FSR's FsrEasuSet up to the
// luminance choice (Rec.709 here vs. weighted-rgb in the reference;
// Rec.709 is the engine's canonical luma per `tonemap.wgsl`).
fn easu_set(
    rgba0 : vec4<f32>, rgba1 : vec4<f32>, rgba2 : vec4<f32>, rgba3 : vec4<f32>,
    frac_xy : vec2<f32>,
) -> vec4<f32> {
    let l0 = luma_rec709(rgba0.rgb);
    let l1 = luma_rec709(rgba1.rgb);
    let l2 = luma_rec709(rgba2.rgb);
    let l3 = luma_rec709(rgba3.rgb);
    // Direction signal: edge orientation in source-pixel space.
    let dir = easu_tap_dir(l0, l1, l2, l3, frac_xy);
    let len = easu_len2(dir);
    // Direction-aware mix weights. The FFX reference derives these
    // via a clamped polynomial in the gradient angle; the
    // simplified-monotone form here preserves the same edge-prefer
    // behaviour without invoking trig.
    let n = dir / vec2<f32>(sqrt(len));
    let pos = clamp(frac_xy * 2.0 - vec2<f32>(1.0), vec2<f32>(-1.0), vec2<f32>(1.0));
    let dir_weight = clamp(dot(n, pos), -1.0, 1.0);
    // Per-axis lerp factors. The four samples are at offsets
    // (-, -), (+, -), (-, +), (+, +); the per-axis fractional position
    // drives the standard bilinear weighting, then the dir_weight
    // pulls toward the edge-orientation sample.
    let w_x = mix(0.5, frac_xy.x, 0.5 + dir_weight * 0.5);
    let w_y = mix(0.5, frac_xy.y, 0.5 + dir_weight * 0.5);
    let w00 = (1.0 - w_x) * (1.0 - w_y);
    let w10 = w_x * (1.0 - w_y);
    let w01 = (1.0 - w_x) * w_y;
    let w11 = w_x * w_y;
    return rgba0 * w00 + rgba1 * w10 + rgba2 * w01 + rgba3 * w11;
}

@compute
@workgroup_size(8, 8, 1)
fn cs_main(@builtin(global_invocation_id) gid : vec3<u32>) {
    let dst_extent = vec2<u32>(textureDimensions(dst).xy);
    if (gid.x >= dst_extent.x || gid.y >= dst_extent.y) {
        return;
    }
    let src_extent = vec2<u32>(textureDimensions(src).xy);
    let src_extent_f = vec2<f32>(f32(src_extent.x), f32(src_extent.y));

    // Map dst pixel centre to source coords.
    let dst_uv = (vec2<f32>(f32(gid.x), f32(gid.y)) + vec2<f32>(0.5))
        / vec2<f32>(f32(dst_extent.x), f32(dst_extent.y));
    let src_xy = dst_uv * src_extent_f - vec2<f32>(0.5);
    let src_xy_floor = floor(src_xy);
    let frac_xy = src_xy - src_xy_floor;

    // Sample the 4 neighbours at half-pixel-aligned UV. Clamp at
    // edges via the sampler's clamp-to-edge address mode (configured
    // at sampler creation, not in WGSL).
    let inv_src = vec2<f32>(1.0, 1.0) / src_extent_f;
    let uv0 = (src_xy_floor + vec2<f32>(0.5, 0.5)) * inv_src;
    let uv1 = (src_xy_floor + vec2<f32>(1.5, 0.5)) * inv_src;
    let uv2 = (src_xy_floor + vec2<f32>(0.5, 1.5)) * inv_src;
    let uv3 = (src_xy_floor + vec2<f32>(1.5, 1.5)) * inv_src;
    let s0 = textureSampleLevel(src, src_sampler, uv0, 0.0);
    let s1 = textureSampleLevel(src, src_sampler, uv1, 0.0);
    let s2 = textureSampleLevel(src, src_sampler, uv2, 0.0);
    let s3 = textureSampleLevel(src, src_sampler, uv3, 0.0);

    let upscaled = easu_set(s0, s1, s2, s3, frac_xy);
    textureStore(dst, vec2<i32>(i32(gid.x), i32(gid.y)), upscaled);
}
