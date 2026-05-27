// IblPass — L2 SH probe evaluation + split-sum BRDF (ADR-065 §2).
//
// Workgroup: (8, 8, 1). Per-pixel evaluates the nearest L2 SH probe
// from the IblProbeSet SSBO, samples the BRDF LUT for specular, and
// adds the result to the lighting accumulation target.
//
// Source-of-truth: `engine_raster::ibl::IblProbeSet::sample`.

struct IblUniforms {
    inv_view_projection : mat4x4<f32>,
    probe_count : u32,
    cell_size_m : f32,
    reserved : vec2<f32>,
};

struct IblProbeRecord {
    cell_key : vec3<i32>,
    pad : u32,
    sh_coeffs : array<vec4<f32>, 9>,    // 9 × L2 SH (xyz RGB, w ignored)
};

@group(1) @binding(0) var<uniform> ibl_uniforms : IblUniforms;
@group(1) @binding(1) var<storage, read> probes : array<IblProbeRecord>;
@group(2) @binding(0) var gbuf_albedo_rough : texture_2d<f32>;
@group(2) @binding(1) var gbuf_normal_metal : texture_2d<f32>;
@group(2) @binding(2) var depth_buffer : texture_depth_2d;
@group(2) @binding(3) var brdf_lut : texture_2d<f32>;
@group(2) @binding(4) var brdf_sampler : sampler;
@group(2) @binding(5) var ibl_out : texture_storage_2d<rgba16float, write>;

fn world_pos_from_depth(uv : vec2<f32>, depth : f32) -> vec3<f32> {
    let ndc = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth, 1.0);
    let world = ibl_uniforms.inv_view_projection * ndc;
    return world.xyz / max(world.w, 1e-6);
}

// Ramamoorthi-Hanrahan L2 SH evaluation. Returns the per-channel
// diffuse irradiance for a given world normal.
fn evaluate_sh(probe_idx : u32, n : vec3<f32>) -> vec3<f32> {
    let p = probes[probe_idx];
    // Cosine-lobe convolved L2 SH basis coefficients (Ramamoorthi-
    // Hanrahan 2001 §3).
    let c1 = 0.429043;
    let c2 = 0.511664;
    let c3 = 0.743125;
    let c4 = 0.886227;
    let c5 = 0.247708;
    let x = n.x;
    let y = n.y;
    let z = n.z;
    // sh_coeffs slot 0 = L00; 1..3 = L1-1 / L10 / L11;
    // 4..8 = L2-2 / L2-1 / L20 / L21 / L22.
    var color = c4 * p.sh_coeffs[0].xyz;
    color = color + 2.0 * c2 * (p.sh_coeffs[3].xyz * x + p.sh_coeffs[1].xyz * y + p.sh_coeffs[2].xyz * z);
    color = color + c1 * (
        p.sh_coeffs[8].xyz * (x * x - y * y)
        + 2.0 * (p.sh_coeffs[4].xyz * x * y + p.sh_coeffs[5].xyz * y * z + p.sh_coeffs[7].xyz * x * z)
    );
    color = color + c3 * p.sh_coeffs[6].xyz * (3.0 * z * z - 1.0) - c5 * p.sh_coeffs[6].xyz;
    return max(color, vec3<f32>(0.0));
}

fn nearest_probe(world_pos : vec3<f32>) -> u32 {
    // Hash to the containing cell key, then linear scan for the
    // matching probe. The CPU oracle uses 8-neighbour trilinear
    // interpolation; PR 4.5 ships the simpler nearest-cell variant
    // that the runner-side PR can extend to trilinear without
    // changing the SSBO layout.
    let cell = vec3<i32>(floor(world_pos / ibl_uniforms.cell_size_m));
    for (var i = 0u; i < ibl_uniforms.probe_count; i = i + 1u) {
        if (all(probes[i].cell_key == cell)) {
            return i;
        }
    }
    return 0u;
}

@compute @workgroup_size(8, 8, 1)
fn cs_main(@builtin(global_invocation_id) gid : vec3<u32>) {
    let dim = textureDimensions(ibl_out);
    if (gid.x >= dim.x || gid.y >= dim.y) {
        return;
    }
    let uv = (vec2<f32>(gid.xy) + vec2<f32>(0.5)) / vec2<f32>(dim);
    let coord = vec2<i32>(uv * vec2<f32>(textureDimensions(depth_buffer)));
    let depth = textureLoad(depth_buffer, coord, 0);
    if (depth <= 0.0) {
        textureStore(ibl_out, vec2<i32>(gid.xy), vec4<f32>(0.0, 0.0, 0.0, 1.0));
        return;
    }
    let world_pos = world_pos_from_depth(uv, depth);
    let n = normalize(textureLoad(gbuf_normal_metal, coord, 0).xyz);
    let albedo_rough = textureLoad(gbuf_albedo_rough, coord, 0);
    let normal_metal = textureLoad(gbuf_normal_metal, coord, 0);
    let metallic = normal_metal.a;
    let roughness = max(albedo_rough.a, 0.04);
    let base_color = albedo_rough.rgb;

    let probe = nearest_probe(world_pos);
    let irradiance = evaluate_sh(probe, n);
    let diffuse = base_color * irradiance * (1.0 - metallic);

    // Split-sum specular sample. The runner-side PR fills in the
    // pre-filtered environment cubemap sampling; PR 4.5 ships the
    // analytical-LUT lookup half.
    let n_dot_v = clamp(dot(n, normalize(-world_pos)), 0.0, 1.0);
    let env_brdf = textureSampleLevel(brdf_lut, brdf_sampler, vec2<f32>(n_dot_v, roughness), 0.0).rg;
    let f0 = mix(vec3<f32>(0.04), base_color, metallic);
    let specular = (f0 * env_brdf.x + vec3<f32>(env_brdf.y)) * irradiance;

    textureStore(ibl_out, vec2<i32>(gid.xy), vec4<f32>(diffuse + specular, 1.0));
}
