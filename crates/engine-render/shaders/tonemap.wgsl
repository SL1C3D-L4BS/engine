// TonemapPass — ACES filmic (Stephen Hill fit) (ADR-065 §6).
//
// Workgroup: (8, 8, 1). Reads TaaResolvedColor + BloomTexture;
// writes TonemappedColor (Bgra8Unorm storage, with manual linear → sRGB
// encoding before `textureStore` since storage-texture writes bypass
// the swapchain view's implicit format conversion).
//
// Source-of-truth: `engine_raster::post_fx::aces_filmic_tonemap`.

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

// ACES input transform.
fn aces_input(v : vec3<f32>) -> vec3<f32> {
    let m = mat3x3<f32>(
        vec3<f32>(0.59719, 0.07600, 0.02840),
        vec3<f32>(0.35458, 0.90834, 0.13383),
        vec3<f32>(0.04823, 0.01566, 0.83777),
    );
    return m * v;
}

// ACES RRT + ODT fit (Stephen Hill).
fn rrt_odt_fit(v : vec3<f32>) -> vec3<f32> {
    let a = v * (v + vec3<f32>(0.0245786)) - vec3<f32>(0.000090537);
    let b = v * (0.983729 * v + vec3<f32>(0.4329510)) + vec3<f32>(0.238081);
    return a / b;
}

// ACES output transform.
fn aces_output(v : vec3<f32>) -> vec3<f32> {
    let m = mat3x3<f32>(
        vec3<f32>(1.60475, -0.10208, -0.00327),
        vec3<f32>(-0.53108, 1.10813, -0.07276),
        vec3<f32>(-0.07367, -0.00605, 1.07602),
    );
    return m * v;
}

fn aces_filmic(v : vec3<f32>) -> vec3<f32> {
    return clamp(aces_output(rrt_odt_fit(aces_input(v))), vec3<f32>(0.0), vec3<f32>(1.0));
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
    let mixed = mix(scene, scene + bloom_sample, tonemap.bloom_mix);
    let exposed = mixed * tonemap.exposure / max(tonemap.white_point, 1e-4);
    let mapped = aces_filmic(exposed);
    // Storage-texture writes bypass the swapchain view's implicit
    // linear → sRGB conversion; encode manually so the displayed
    // image is perceptually correct.
    let encoded = linear_to_srgb(mapped);
    textureStore(dst, vec2<i32>(gid.xy), vec4<f32>(encoded, 1.0));
}
