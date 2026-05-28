// LightingAccumulationPass — Cook-Torrance GGX / Smith-Schlick
// over cluster lights + CSM (ADR-064 §6).
//
// Full-screen triangle (no vertex buffer). Reads G-buffer slots
// 0/1/2 + DepthBuffer + ClusterCells + LightData + ShadowAtlas;
// writes `LitColor` (Rgba16Float).
//
// Source-of-truth: `engine_raster::shading::cook_torrance_ggx`.

struct FullScreenUniforms {
    inv_view_projection : mat4x4<f32>,
    camera_pos : vec4<f32>,
    screen_extent : vec2<f32>,
    pad : vec2<f32>,
};

struct LightRecord {
    position_radius : vec4<f32>,
    color_intensity : vec4<f32>,
    direction : vec4<f32>,
    params : vec4<f32>,
};

struct ClusterCell {
    light_offset : u32,
    light_count : u32,
};

struct ClusterUniforms {
    inv_view_projection : mat4x4<f32>,
    light_count : u32,
    grid_dim : vec3<u32>,
    z_near : f32,
    z_far : f32,
    reserved : vec2<f32>,
};

// Mirrors `CsmUniforms` in `csm_shadow.wgsl` + `engine_raster::shadow::Cascade`.
// Phase 6 PR 1b (ADR-040 + ADR-081): the lighting pass now binds the
// same UBO the shadow-fill pass writes through, so cascade NDC projection
// + atlas-quadrant sampling happen against bit-identical matrices on
// both passes. Closes the `csm_4_cascade` oracle exception.
struct CsmUniforms {
    cascade_vp : array<mat4x4<f32>, 4>,
    cascade_split_far : array<vec4<f32>, 1>,
    filter_radius_px : f32,
    bias_constant : f32,
    bias_slope : f32,
};

@group(0) @binding(0) var<uniform> frame : FullScreenUniforms;
@group(1) @binding(0) var<uniform> cluster : ClusterUniforms;
@group(1) @binding(1) var<storage, read> lights : array<LightRecord>;
@group(1) @binding(2) var<storage, read> cells : array<ClusterCell>;
@group(1) @binding(3) var<storage, read> light_indices : array<u32>;
@group(1) @binding(4) var<uniform> csm : CsmUniforms;
@group(2) @binding(0) var gbuf_albedo_rough : texture_2d<f32>;
@group(2) @binding(1) var gbuf_normal_metal : texture_2d<f32>;
@group(2) @binding(2) var gbuf_motion_depth : texture_2d<f32>;
@group(2) @binding(3) var depth_buffer : texture_depth_2d;
@group(2) @binding(4) var shadow_atlas : texture_depth_2d;
@group(2) @binding(5) var shadow_sampler : sampler_comparison;

const PI : f32 = 3.14159265358979;

// Source-of-truth: `engine_raster::shading::cook_torrance` (CPU oracle).
//
// "Roughness" is the perceptual UE/Disney convention. The GGX α is the
// inner square (α = roughness²); α² in the NDF is therefore roughness⁴.
// Smith-Schlick's k follows GGX's α, not the perceptual roughness, so
// k = (α + 1)² / 8 = (roughness² + 1)² / 8.
//
// Earlier revisions of this shader collapsed both squares into one
// (`a2 = roughness²`, `k = (roughness + 1)² / 8`), which silently
// reparameterised the BRDF — fragments with a CPU-authored roughness
// of 0.35 would specular-peak as if they were ~0.59 on the GPU. Pixel
// parity surfaced the drift; aligning the two formulae closes the gap.
fn ggx_d(n_dot_h : f32, roughness : f32) -> f32 {
    let alpha = roughness * roughness;
    let alpha2 = alpha * alpha;
    let denom = n_dot_h * n_dot_h * (alpha2 - 1.0) + 1.0;
    return alpha2 / max(PI * denom * denom, 1e-6);
}

fn smith_g1(n_dot_v : f32, k : f32) -> f32 {
    return n_dot_v / (n_dot_v * (1.0 - k) + k);
}

fn smith_g(n_dot_v : f32, n_dot_l : f32, roughness : f32) -> f32 {
    let alpha = roughness * roughness;
    let r = alpha + 1.0;
    let k = (r * r) / 8.0;
    return smith_g1(n_dot_v, k) * smith_g1(n_dot_l, k);
}

fn schlick_f(v_dot_h : f32, f0 : vec3<f32>) -> vec3<f32> {
    return f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - v_dot_h, 5.0);
}

fn cook_torrance(
    base_color : vec3<f32>,
    metallic : f32,
    roughness : f32,
    n : vec3<f32>,
    v : vec3<f32>,
    l : vec3<f32>,
    light_color : vec3<f32>,
    light_intensity : f32,
) -> vec3<f32> {
    let h = normalize(v + l);
    let n_dot_l = max(dot(n, l), 0.0);
    let n_dot_v = max(dot(n, v), 1e-4);
    let n_dot_h = max(dot(n, h), 0.0);
    let v_dot_h = max(dot(v, h), 0.0);
    let f0 = mix(vec3<f32>(0.04), base_color, metallic);
    let d = ggx_d(n_dot_h, roughness);
    let g = smith_g(n_dot_v, n_dot_l, roughness);
    let f = schlick_f(v_dot_h, f0);
    let specular = (d * g * f) / max(4.0 * n_dot_v * n_dot_l, 1e-4);
    let kd = (vec3<f32>(1.0) - f) * (1.0 - metallic);
    let diffuse = kd * base_color / PI;
    return (diffuse + specular) * light_color * light_intensity * n_dot_l;
}

struct VsOut {
    @builtin(position) clip : vec4<f32>,
    @location(0) uv : vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) idx : u32) -> VsOut {
    // Full-screen triangle from {(-1,-1), (3,-1), (-1,3)} NDC.
    var pos = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    var uv = array<vec2<f32>, 3>(
        vec2<f32>(0.0, 1.0),
        vec2<f32>(2.0, 1.0),
        vec2<f32>(0.0, -1.0),
    );
    var out : VsOut;
    out.clip = vec4<f32>(pos[idx], 0.0, 1.0);
    out.uv = uv[idx];
    return out;
}

fn world_pos_from_depth(uv : vec2<f32>, depth : f32) -> vec3<f32> {
    let ndc = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth, 1.0);
    let world = frame.inv_view_projection * ndc;
    return world.xyz / max(world.w, 1e-6);
}

// CSM cascade selection (ADR-040 + ADR-081 §1). Picks the smallest
// cascade whose split-far still covers `view_z`. Mirrors the CPU
// oracle `engine_raster::shadow::select_cascade`. View distance is
// the chord distance from the camera — a small approximation vs the
// rasterizer's view-space z (forward-axis dot), but cascade slabs
// are radial w.r.t. the camera, so the boundary error stays below
// one cascade's PCF kernel width.
fn select_cascade(view_z : f32) -> u32 {
    let splits = csm.cascade_split_far[0];
    if (view_z <= splits.x) { return 0u; }
    if (view_z <= splits.y) { return 1u; }
    if (view_z <= splits.z) { return 2u; }
    return 3u;
}

// 16-tap Vogel-disk PCF, golden-angle rotation, 5×5 footprint.
// Mirrors `engine_raster::shadow::sample_shadow_pcf` and matches the
// pixel-parity fixture's golden image. Returns `1.0` for fully lit,
// `0.0` for fully shadowed, intermediate for partial occlusion.
fn sample_shadow_pcf(cascade_idx : u32, world_p : vec3<f32>) -> f32 {
    let vp = csm.cascade_vp[cascade_idx];
    let clip = vp * vec4<f32>(world_p, 1.0);
    if (abs(clip.w) < 1e-6) { return 1.0; }
    let ndc = clip.xyz / clip.w;
    if (ndc.x < -1.0 || ndc.x > 1.0 || ndc.y < -1.0 || ndc.y > 1.0) {
        return 1.0;
    }
    // CASCADE_DIM = ATLAS_DIM / 2 = 2048 (per `engine_raster::shadow`).
    let cascade_dim : f32 = 2048.0;
    let atlas_dim : f32 = 4096.0;
    let texel : f32 = 1.0 / cascade_dim;
    // Map cascade NDC.xy to UV inside the cascade's atlas quadrant.
    // Quadrant origin (ADR-040 §2 fixed layout):
    //   cascade 0: (0, 0), cascade 1: (0.5, 0),
    //   cascade 2: (0, 0.5), cascade 3: (0.5, 0.5)  — in [0,1] UV.
    let q_u_offset : f32 = f32(cascade_idx & 1u) * 0.5;
    let q_v_offset : f32 = f32((cascade_idx >> 1u) & 1u) * 0.5;
    let cascade_u = ndc.x * 0.5 + 0.5;
    let cascade_v = 1.0 - (ndc.y * 0.5 + 0.5);
    let fragment_depth = ndc.z;
    // 16 Vogel-disk samples, golden-angle rotation. The fixed pattern
    // matches the CPU oracle (per-pixel rotation by a screen-space
    // hash is a future GPU tweak; the oracle locks the average).
    let golden_angle : f32 = 2.3999632;
    let radius : f32 = 2.0;
    var accum : f32 = 0.0;
    for (var k : u32 = 0u; k < 16u; k = k + 1u) {
        let r = sqrt((f32(k) + 0.5) / 16.0) * radius;
        let theta = f32(k) * golden_angle;
        let dx = cos(theta) * r * texel;
        let dy = sin(theta) * r * texel;
        let sample_u = q_u_offset + clamp(cascade_u + dx, 0.0, 1.0) * 0.5;
        let sample_v = q_v_offset + clamp(cascade_v + dy, 0.0, 1.0) * 0.5;
        // Reverse-Z + small acne bias matches the CPU oracle's
        // `bias = 1e-4`. The hardware comparison sampler is set up
        // with `Greater` (Phase 5.5 A.2c discipline); the bias goes
        // on the *reference* (fragment) depth so a near-tangential
        // surface stays unshadowed.
        let sample_ref = fragment_depth + 1e-4;
        accum = accum + textureSampleCompareLevel(
            shadow_atlas, shadow_sampler,
            vec2<f32>(sample_u, sample_v),
            sample_ref,
        );
    }
    return accum / 16.0;
}

@fragment
fn fs_main(in : VsOut) -> @location(0) vec4<f32> {
    let coord = vec2<i32>(in.uv * frame.screen_extent);
    let albedo_rough = textureLoad(gbuf_albedo_rough, coord, 0);
    let normal_metal = textureLoad(gbuf_normal_metal, coord, 0);
    let depth = textureLoad(depth_buffer, coord, 0);
    // Reference declared bindings PR 4.5 left for the auto-derived
    // layout's sake. Naga reflection strips declared-but-unused
    // bindings; touching them keeps the layout in sync with the
    // record-body bind-group descriptor.
    let _motion = textureLoad(gbuf_motion_depth, coord, 0);
    if (depth <= 0.0) {
        // Reverse-Z: 0.0 is far / sky. Don't shade.
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    let world_pos = world_pos_from_depth(in.uv, depth);
    let n = normalize(normal_metal.xyz);
    let v = normalize(frame.camera_pos.xyz - world_pos);
    let base_color = albedo_rough.rgb;
    let roughness = max(albedo_rough.a, 0.04);
    let metallic = normal_metal.a;

    var color = vec3<f32>(0.0);
    // Cascade-select once per fragment (ADR-040 §3). Mirrors the CPU
    // oracle `engine_raster::shading::accumulate_lighting`: chord
    // distance to camera, indexed against the 4-element split table.
    let view_distance = length(world_pos - frame.camera_pos.xyz);
    let cascade_idx = select_cascade(view_distance);
    let shadow_visibility = sample_shadow_pcf(cascade_idx, world_pos);
    // Walk the cluster cell at this fragment. The cluster lookup
    // mirrors the CPU oracle `engine_raster::cluster::cell_for_view_pos`.
    let view_depth = -depth; // reverse-Z → view depth
    let view_z_norm = log(max(-view_depth, cluster.z_near) / cluster.z_near)
        / log(cluster.z_far / cluster.z_near);
    let cell_x = u32(clamp(in.uv.x * f32(cluster.grid_dim.x), 0.0,
        f32(cluster.grid_dim.x - 1u)));
    let cell_y = u32(clamp(in.uv.y * f32(cluster.grid_dim.y), 0.0,
        f32(cluster.grid_dim.y - 1u)));
    let cell_z = u32(clamp(view_z_norm * f32(cluster.grid_dim.z), 0.0,
        f32(cluster.grid_dim.z - 1u)));
    let cell_idx = cell_z * cluster.grid_dim.x * cluster.grid_dim.y
        + cell_y * cluster.grid_dim.x + cell_x;
    let cell = cells[cell_idx];
    for (var i = 0u; i < cell.light_count; i = i + 1u) {
        let li = light_indices[cell.light_offset + i];
        let light = lights[li];
        let r = light.position_radius.w;
        // Light-type branch (ADR-064 §5):
        //   r > 0 → point (or spot, treated as point here until the
        //           cone-falloff lands)
        //   r <= 0 → directional. The light's stored
        //           `direction.xyz` points light → scene; the BRDF's
        //           `l` input is surface → light, hence the negation.
        //           Source-of-truth: `engine_raster::sample::CubeParityScene::render_cpu`,
        //           which does the same negation.
        let to_light = select(
            -light.direction.xyz,
            light.position_radius.xyz - world_pos,
            r > 0.0,
        );
        let dist_sq = dot(to_light, to_light);
        if (r > 0.0 && dist_sq > r * r) {
            continue;
        }
        let l = normalize(to_light);
        // Point-light attenuation matches the CPU oracle
        // `engine_raster::shading::light_dir_and_attenuation`: a
        // windowed inverse-square with an end-of-range cutoff:
        //   t  = clamp(1 - dist/range, 0, 1)
        //   att = (t·t) / dist²
        // ADR-043 + ADR-081 §1: aligns lighting.wgsl with the
        // attenuation kernel the rasterizer reference uses. Closes
        // the `cluster_64_lights` post-v0.3 exception to strict 1/255.
        let dist_p = sqrt(max(dist_sq, 1e-12));
        let t_win = select(0.0, clamp(1.0 - dist_p / r, 0.0, 1.0), r > 0.0);
        let attenuation = select(1.0, (t_win * t_win) / max(dist_sq, 1e-6), r > 0.0);
        // CSM shadow visibility applies only to directional lights
        // (r <= 0). Point lights default to fully lit until the
        // point-light shadow-map pass lands (Phase 7+ scope).
        let visibility = select(shadow_visibility, 1.0, r > 0.0);
        color = color + cook_torrance(
            base_color, metallic, roughness, n, v, l,
            light.color_intensity.rgb, light.color_intensity.a * attenuation * visibility,
        );
    }
    return vec4<f32>(color, 1.0);
}
