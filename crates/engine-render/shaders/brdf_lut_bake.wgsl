// BrdfLutBake — one-shot BRDF LUT bake (ADR-065 §3).
//
// Compute pass executed once at engine init. Bakes the split-sum
// specular BRDF LUT into a 512² Rg16Float texture. Subsequent runs
// load from $XDG_CACHE_HOME/sliced-engine/brdf_lut.bin.
//
// Source-of-truth: `engine_raster::ibl::bake_brdf_lut`.

const BRDF_LUT_SAMPLES : u32 = 1024u;
const PI : f32 = 3.14159265358979;

@group(0) @binding(0) var lut_out : texture_storage_2d<rg16float, write>;

fn radical_inverse_vdc(bits_in : u32) -> f32 {
    var bits = bits_in;
    bits = (bits << 16u) | (bits >> 16u);
    bits = ((bits & 0x55555555u) << 1u) | ((bits & 0xAAAAAAAAu) >> 1u);
    bits = ((bits & 0x33333333u) << 2u) | ((bits & 0xCCCCCCCCu) >> 2u);
    bits = ((bits & 0x0F0F0F0Fu) << 4u) | ((bits & 0xF0F0F0F0u) >> 4u);
    bits = ((bits & 0x00FF00FFu) << 8u) | ((bits & 0xFF00FF00u) >> 8u);
    return f32(bits) * 2.3283064365386963e-10;
}

fn hammersley(i : u32, n : u32) -> vec2<f32> {
    return vec2<f32>(f32(i) / f32(n), radical_inverse_vdc(i));
}

fn ggx_importance_sample(xi : vec2<f32>, n : vec3<f32>, roughness : f32) -> vec3<f32> {
    let a = roughness * roughness;
    let phi = 2.0 * PI * xi.x;
    let cos_theta = sqrt((1.0 - xi.y) / (1.0 + (a * a - 1.0) * xi.y));
    let sin_theta = sqrt(1.0 - cos_theta * cos_theta);
    let h = vec3<f32>(cos(phi) * sin_theta, sin(phi) * sin_theta, cos_theta);
    let up = select(vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(1.0, 0.0, 0.0),
        abs(n.z) < 0.999);
    let tangent = normalize(cross(up, n));
    let bitangent = cross(n, tangent);
    return normalize(tangent * h.x + bitangent * h.y + n * h.z);
}

fn smith_g_correlated(n_dot_v : f32, n_dot_l : f32, roughness : f32) -> f32 {
    let r = roughness;
    let k = (r * r) / 2.0;
    let gv = n_dot_v / (n_dot_v * (1.0 - k) + k);
    let gl = n_dot_l / (n_dot_l * (1.0 - k) + k);
    return gv * gl;
}

fn integrate_brdf(n_dot_v : f32, roughness : f32) -> vec2<f32> {
    let v = vec3<f32>(sqrt(1.0 - n_dot_v * n_dot_v), 0.0, n_dot_v);
    let n = vec3<f32>(0.0, 0.0, 1.0);
    var a = 0.0;
    var b = 0.0;
    for (var i = 0u; i < BRDF_LUT_SAMPLES; i = i + 1u) {
        let xi = hammersley(i, BRDF_LUT_SAMPLES);
        let h = ggx_importance_sample(xi, n, roughness);
        let l = normalize(2.0 * dot(v, h) * h - v);
        let n_dot_l = max(l.z, 0.0);
        let n_dot_h = max(h.z, 0.0);
        let v_dot_h = max(dot(v, h), 0.0);
        if (n_dot_l > 0.0) {
            let g = smith_g_correlated(n_dot_v, n_dot_l, roughness);
            let g_vis = (g * v_dot_h) / max(n_dot_h * n_dot_v, 1e-4);
            let fc = pow(1.0 - v_dot_h, 5.0);
            a = a + (1.0 - fc) * g_vis;
            b = b + fc * g_vis;
        }
    }
    return vec2<f32>(a, b) / f32(BRDF_LUT_SAMPLES);
}

@compute @workgroup_size(8, 8, 1)
fn cs_main(@builtin(global_invocation_id) gid : vec3<u32>) {
    let dim = textureDimensions(lut_out);
    if (gid.x >= dim.x || gid.y >= dim.y) {
        return;
    }
    let n_dot_v = (f32(gid.x) + 0.5) / f32(dim.x);
    let roughness = max((f32(gid.y) + 0.5) / f32(dim.y), 0.04);
    let scale_bias = integrate_brdf(n_dot_v, roughness);
    textureStore(lut_out, vec2<i32>(gid.xy), vec4<f32>(scale_bias, 0.0, 0.0));
}
