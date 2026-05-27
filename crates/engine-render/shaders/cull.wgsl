// CullPass — compute frustum cull (ADR-064 §7).
//
// Workgroup: (64, 1, 1) per `contracts::CULL_WORKGROUP_SIZE`. One
// thread per instance-batch entry: compares the entry's world-space
// AABB against the 6 frustum planes and appends the surviving
// instance's draw command into IndirectDrawBuffer.
//
// Source-of-truth math: `engine_raster::scene::Frustum::contains_aabb`.

struct Aabb {
    min : vec3<f32>,
    pad0 : f32,
    max : vec3<f32>,
    pad1 : f32,
};

struct InstanceEntry {
    aabb : Aabb,
    mesh_index : u32,
    material_index : u32,
    instance_id : u32,
    flags : u32,
};

struct DrawIndirect {
    index_count : u32,
    instance_count : u32,
    first_index : u32,
    base_vertex : i32,
    first_instance : u32,
};

struct MeshEntry {
    index_count : u32,
    first_index : u32,
    base_vertex : i32,
    flags : u32,
};

struct Frustum {
    // 6 planes: left, right, bottom, top, near, far.
    // Each plane: xyz = normal, w = signed distance from origin.
    planes : array<vec4<f32>, 6>,
};

@group(0) @binding(0) var<uniform> frustum : Frustum;
@group(0) @binding(1) var<storage, read> instances : array<InstanceEntry>;
@group(0) @binding(2) var<storage, read> meshes : array<MeshEntry>;
@group(0) @binding(3) var<storage, read_write> draws : array<DrawIndirect>;
@group(0) @binding(4) var<storage, read_write> draw_count : array<atomic<u32>, 1>;

fn aabb_outside_plane(aabb : Aabb, plane : vec4<f32>) -> bool {
    // Vertex of the AABB farthest along the plane normal.
    let n_pos = vec3<f32>(
        select(aabb.min.x, aabb.max.x, plane.x >= 0.0),
        select(aabb.min.y, aabb.max.y, plane.y >= 0.0),
        select(aabb.min.z, aabb.max.z, plane.z >= 0.0),
    );
    return dot(plane.xyz, n_pos) + plane.w < 0.0;
}

fn aabb_inside_frustum(aabb : Aabb) -> bool {
    for (var i = 0u; i < 6u; i = i + 1u) {
        if (aabb_outside_plane(aabb, frustum.planes[i])) {
            return false;
        }
    }
    return true;
}

@compute @workgroup_size(64, 1, 1)
fn cs_main(@builtin(global_invocation_id) gid : vec3<u32>) {
    let idx = gid.x;
    if (idx >= arrayLength(&instances)) {
        return;
    }
    let entry = instances[idx];
    if (!aabb_inside_frustum(entry.aabb)) {
        return;
    }
    let mesh = meshes[entry.mesh_index];
    let slot = atomicAdd(&draw_count[0], 1u);
    draws[slot] = DrawIndirect(
        mesh.index_count,
        1u,
        mesh.first_index,
        mesh.base_vertex,
        entry.instance_id,
    );
}
