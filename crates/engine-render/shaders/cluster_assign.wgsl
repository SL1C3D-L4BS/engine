// ClusterLightPass — compute light assignment per cluster cell
// (ADR-043 + ADR-064 §5).
//
// Workgroup: (16, 9, 1) per `contracts::CLUSTER_ASSIGN_WORKGROUP_SIZE`,
// matching the cluster grid's X+Y tile counts. Each thread walks the
// Z slices internally (24 of them) and the global light list
// (cap 256) inserting hits into a per-cell list capped at 32 lights.
//
// Source-of-truth: `engine_raster::cluster::assign_lights`.

struct ClusterUniforms {
    inv_view_projection : mat4x4<f32>,
    light_count : u32,
    grid_dim : vec3<u32>,
    z_near : f32,
    z_far : f32,
    reserved : vec2<f32>,
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

@group(1) @binding(0) var<uniform> cluster_uniforms : ClusterUniforms;
@group(1) @binding(1) var<storage, read> lights : array<LightRecord>;
@group(1) @binding(2) var<storage, read_write> cells : array<ClusterCell>;
@group(1) @binding(3) var<storage, read_write> light_indices : array<u32>;
@group(1) @binding(4) var<storage, read_write> indices_cursor : array<atomic<u32>, 1>;

const MAX_LIGHTS_PER_CLUSTER : u32 = 32u;

fn cluster_view_aabb(cell : vec3<u32>) -> array<vec3<f32>, 2> {
    // Logarithmic Z slice; linear X/Y NDC slice.
    let nx = f32(cell.x) / f32(cluster_uniforms.grid_dim.x);
    let nx1 = f32(cell.x + 1u) / f32(cluster_uniforms.grid_dim.x);
    let ny = f32(cell.y) / f32(cluster_uniforms.grid_dim.y);
    let ny1 = f32(cell.y + 1u) / f32(cluster_uniforms.grid_dim.y);
    let zn = cluster_uniforms.z_near * pow(
        cluster_uniforms.z_far / cluster_uniforms.z_near,
        f32(cell.z) / f32(cluster_uniforms.grid_dim.z),
    );
    let zf = cluster_uniforms.z_near * pow(
        cluster_uniforms.z_far / cluster_uniforms.z_near,
        f32(cell.z + 1u) / f32(cluster_uniforms.grid_dim.z),
    );

    // NDC corners projected to view space via inv_view_projection.
    var min_v = vec3<f32>(1e30, 1e30, 1e30);
    var max_v = vec3<f32>(-1e30, -1e30, -1e30);
    for (var i = 0u; i < 8u; i = i + 1u) {
        let xn = select(nx, nx1, (i & 1u) != 0u) * 2.0 - 1.0;
        let yn = select(ny, ny1, (i & 2u) != 0u) * 2.0 - 1.0;
        let zv = select(zn, zf, (i & 4u) != 0u);
        // Reconstruct view-space position from NDC + view depth.
        let view = vec3<f32>(xn * zv, yn * zv, -zv);
        min_v = min(min_v, view);
        max_v = max(max_v, view);
    }
    return array<vec3<f32>, 2>(min_v, max_v);
}

fn light_intersects_aabb(light : LightRecord, aabb_min : vec3<f32>, aabb_max : vec3<f32>) -> bool {
    let radius = light.position_radius.w;
    if (radius <= 0.0) {
        // Directional light — intersects every cluster.
        return true;
    }
    let center = light.position_radius.xyz;
    let closest = clamp(center, aabb_min, aabb_max);
    let d = center - closest;
    return dot(d, d) <= radius * radius;
}

@compute @workgroup_size(16, 9, 1)
fn cs_main(@builtin(global_invocation_id) gid : vec3<u32>) {
    let xy = gid.xy;
    if (xy.x >= cluster_uniforms.grid_dim.x || xy.y >= cluster_uniforms.grid_dim.y) {
        return;
    }
    for (var z = 0u; z < cluster_uniforms.grid_dim.z; z = z + 1u) {
        let aabb = cluster_view_aabb(vec3<u32>(xy, z));
        let cell_idx =
            z * cluster_uniforms.grid_dim.x * cluster_uniforms.grid_dim.y
            + xy.y * cluster_uniforms.grid_dim.x
            + xy.x;

        // Reserve a contiguous slot range in light_indices.
        let max_lights = MAX_LIGHTS_PER_CLUSTER;
        let base_offset = atomicAdd(&indices_cursor[0], max_lights);
        var hit_count = 0u;
        for (var l = 0u; l < cluster_uniforms.light_count; l = l + 1u) {
            if (hit_count >= max_lights) {
                break;
            }
            if (light_intersects_aabb(lights[l], aabb[0], aabb[1])) {
                light_indices[base_offset + hit_count] = l;
                hit_count = hit_count + 1u;
            }
        }
        cells[cell_idx] = ClusterCell(base_offset, hit_count);
    }
}
