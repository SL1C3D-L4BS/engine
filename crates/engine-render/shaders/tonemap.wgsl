// TonemapPass — Narkowicz 2015 ACES fit (ADR-065 §6).
//
// Workgroup: (8, 8, 1). Reads TaaResolvedColor + BloomTexture;
// writes TonemappedColor (Bgra8Unorm storage, with manual linear → sRGB
// encoding before `textureStore` since storage-texture writes bypass
// the swapchain view's implicit format conversion).
//
// Source-of-truth: `engine_raster::post_fx::tonemap_aces`. Earlier
// revisions imported the Hill 2017 RRT/ODT fit with a `white_point`
// divisor of 11.2; aligning to the CPU oracle's curve closed an
// undocumented ~5× brightness mismatch at parity time.

// Per-component linear → sRGB conversion (IEC 61966-2-1).
fn linear_to_srgb_component(c : f32) -> f32 {
    if (c <= 0.0031308) {
        return c * 12.92;
    }
    return 1.055 * pow(c, 1.0 / 2.4) - 0.055;
}

fn linear_to_srgb(rgb : vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        linear_to_srgb_component(rgb.x),
        linear_to_srgb_component(rgb.y),
        linear_to_srgb_component(rgb.z),
    );
}

struct TonemapUniforms {
    exposure : f32,
    bloom_mix : f32,
    white_point : f32,
    reserved : f32,
};

@group(1) @binding(0) var<uniform> tonemap : TonemapUniforms;
@group(2) @binding(0) var taa_resolved : texture_2d<f32>;
@group(2) @binding(1) var bloom : texture_2d<f32>;
@group(2) @binding(2) var lin_sampler : sampler;
@group(2) @binding(3) var dst : texture_storage_2d<bgra8unorm, write>;

// Narkowicz 2015 ACES rational fit, matching the CPU oracle's
// `tonemap_aces`. `white_point` is preserved in the UBO for the bloom
// composition path (intensity scaling), but the tonemap curve itself
// does not divide by it — the CPU oracle does no such divide.
fn aces_narkowicz_component(x : f32) -> f32 {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    let num = x * (a * x + b);
    let den = x * (c * x + d) + e;
    return clamp(num / den, 0.0, 1.0);
}

fn aces_filmic(v : vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        aces_narkowicz_component(max(v.x, 0.0)),
        aces_narkowicz_component(max(v.y, 0.0)),
        aces_narkowicz_component(max(v.z, 0.0)),
    );
}

@compute @workgroup_size(8, 8, 1)
fn cs_main(@builtin(global_invocation_id) gid : vec3<u32>) {
    let dim = textureDimensions(dst);
    if (gid.x >= dim.x || gid.y >= dim.y) {
        return;
    }
    let uv = (vec2<f32>(gid.xy) + vec2<f32>(0.5)) / vec2<f32>(dim);
    let scene = textureSampleLevel(taa_resolved, lin_sampler, uv, 0.0).rgb;
    let bloom_sample = textureSampleLevel(bloom, lin_sampler, uv, 0.0).rgb;
    // Bloom composition is a linear add scaled by `bloom_mix`; the
    // `white_point` UBO field stays available to scale bloom intensity
    // before the curve, matching the CPU oracle's
    // `post_fx::bloom_composite + tonemap_aces` chain.
    _ = tonemap.white_point;
    let mixed = mix(scene, scene + bloom_sample, tonemap.bloom_mix);
    let exposed = mixed * tonemap.exposure;
    let mapped = aces_filmic(exposed);
    // Storage-texture writes bypass the swapchain view's implicit
    // linear → sRGB conversion; encode manually so the displayed
    // image is perceptually correct.
    let encoded = linear_to_srgb(mapped);
    textureStore(dst, vec2<i32>(gid.xy), vec4<f32>(encoded, 1.0));
}
