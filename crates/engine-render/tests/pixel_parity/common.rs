//! Byte-layout + scene-seeding helpers shared across Phase 5.5 A.3
//! parity fixtures.
//!
//! Every fixture writes the same cube geometry, the same frustum + cull
//! input shape, and the same UBO byte layouts; only the *values* that
//! seed the feature-under-test (light list, IBL probe SH coefficients,
//! CSM cascade matrices, etc.) vary per fixture. Hoisting the common
//! writers here keeps each fixture file focused on its scene-specific
//! data without duplicating ~500 LOC of std140 byte plumbing.
//!
//! The cube fixture authored these helpers; subsequent fixtures inherit
//! them verbatim and override only what they care about.

#![allow(dead_code)]

use engine_gpu::Buffer;
use engine_math::Mat4;
use engine_render::{ResourceId, ResourceResolver, TransientResourceTable};

/// Resolve a buffer the harness pool already registered. Panics on
/// missing ids — the pool guarantees every canonical id is populated.
pub fn buffer_for(table: &TransientResourceTable, id: ResourceId) -> &Buffer {
    table
        .resolve_buffer(id)
        .expect("harness registered this id in the pool")
}

// =============================================================================
// Byte-layout helpers — write WGSL `repr(std140)`-compatible records into
// a `Vec<u8>` field-by-field. Avoids `unsafe` on `#[repr(C)]` reinterpret.
// =============================================================================

pub fn push_f32(buf: &mut Vec<u8>, v: f32) {
    buf.extend_from_slice(&v.to_le_bytes());
}

pub fn push_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_le_bytes());
}

pub fn push_i32(buf: &mut Vec<u8>, v: i32) {
    buf.extend_from_slice(&v.to_le_bytes());
}

pub fn push_vec4(buf: &mut Vec<u8>, v: [f32; 4]) {
    for x in v.iter() {
        push_f32(buf, *x);
    }
}

pub fn push_mat4(buf: &mut Vec<u8>, m: Mat4) {
    let cols = m.to_cols_array();
    for v in cols.iter() {
        push_f32(buf, *v);
    }
}

pub fn pad_to(buf: &mut Vec<u8>, target_len: usize) {
    while buf.len() < target_len {
        buf.push(0);
    }
}

// =============================================================================
// Cube mesh — 24 verts (4 per face × 6 faces) for per-face normals,
// 36 indices. Vertex layout per ADR-061: 48 B = pos(12) + normal(12) +
// tangent(16) + uv(8).
// =============================================================================

/// One cube vertex: position + normal + tangent + uv = 48 B.
pub type CubeVert = ([f32; 3], [f32; 3], [f32; 4], [f32; 2]);

pub fn cube_vertex_buffer(aabb_min: [f32; 3], aabb_max: [f32; 3]) -> Vec<u8> {
    let [x0, y0, z0] = aabb_min;
    let [x1, y1, z1] = aabb_max;
    let faces: [[CubeVert; 4]; 6] = [
        // +X
        [
            (
                [x1, y0, z0],
                [1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 1.0],
                [0.0, 0.0],
            ),
            (
                [x1, y1, z0],
                [1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 1.0],
                [1.0, 0.0],
            ),
            (
                [x1, y1, z1],
                [1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 1.0],
                [1.0, 1.0],
            ),
            (
                [x1, y0, z1],
                [1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 1.0],
                [0.0, 1.0],
            ),
        ],
        // -X
        [
            (
                [x0, y0, z1],
                [-1.0, 0.0, 0.0],
                [0.0, 0.0, -1.0, 1.0],
                [0.0, 0.0],
            ),
            (
                [x0, y1, z1],
                [-1.0, 0.0, 0.0],
                [0.0, 0.0, -1.0, 1.0],
                [1.0, 0.0],
            ),
            (
                [x0, y1, z0],
                [-1.0, 0.0, 0.0],
                [0.0, 0.0, -1.0, 1.0],
                [1.0, 1.0],
            ),
            (
                [x0, y0, z0],
                [-1.0, 0.0, 0.0],
                [0.0, 0.0, -1.0, 1.0],
                [0.0, 1.0],
            ),
        ],
        // +Y
        [
            (
                [x0, y1, z0],
                [0.0, 1.0, 0.0],
                [1.0, 0.0, 0.0, 1.0],
                [0.0, 0.0],
            ),
            (
                [x0, y1, z1],
                [0.0, 1.0, 0.0],
                [1.0, 0.0, 0.0, 1.0],
                [1.0, 0.0],
            ),
            (
                [x1, y1, z1],
                [0.0, 1.0, 0.0],
                [1.0, 0.0, 0.0, 1.0],
                [1.0, 1.0],
            ),
            (
                [x1, y1, z0],
                [0.0, 1.0, 0.0],
                [1.0, 0.0, 0.0, 1.0],
                [0.0, 1.0],
            ),
        ],
        // -Y
        [
            (
                [x0, y0, z1],
                [0.0, -1.0, 0.0],
                [1.0, 0.0, 0.0, 1.0],
                [0.0, 0.0],
            ),
            (
                [x0, y0, z0],
                [0.0, -1.0, 0.0],
                [1.0, 0.0, 0.0, 1.0],
                [1.0, 0.0],
            ),
            (
                [x1, y0, z0],
                [0.0, -1.0, 0.0],
                [1.0, 0.0, 0.0, 1.0],
                [1.0, 1.0],
            ),
            (
                [x1, y0, z1],
                [0.0, -1.0, 0.0],
                [1.0, 0.0, 0.0, 1.0],
                [0.0, 1.0],
            ),
        ],
        // +Z
        [
            (
                [x0, y0, z1],
                [0.0, 0.0, 1.0],
                [1.0, 0.0, 0.0, 1.0],
                [0.0, 0.0],
            ),
            (
                [x1, y0, z1],
                [0.0, 0.0, 1.0],
                [1.0, 0.0, 0.0, 1.0],
                [1.0, 0.0],
            ),
            (
                [x1, y1, z1],
                [0.0, 0.0, 1.0],
                [1.0, 0.0, 0.0, 1.0],
                [1.0, 1.0],
            ),
            (
                [x0, y1, z1],
                [0.0, 0.0, 1.0],
                [1.0, 0.0, 0.0, 1.0],
                [0.0, 1.0],
            ),
        ],
        // -Z
        [
            (
                [x1, y0, z0],
                [0.0, 0.0, -1.0],
                [-1.0, 0.0, 0.0, 1.0],
                [0.0, 0.0],
            ),
            (
                [x0, y0, z0],
                [0.0, 0.0, -1.0],
                [-1.0, 0.0, 0.0, 1.0],
                [1.0, 0.0],
            ),
            (
                [x0, y1, z0],
                [0.0, 0.0, -1.0],
                [-1.0, 0.0, 0.0, 1.0],
                [1.0, 1.0],
            ),
            (
                [x1, y1, z0],
                [0.0, 0.0, -1.0],
                [-1.0, 0.0, 0.0, 1.0],
                [0.0, 1.0],
            ),
        ],
    ];

    let mut buf = Vec::with_capacity(48 * 24);
    for face in faces.iter() {
        for (pos, normal, tangent, uv) in face.iter() {
            for v in pos.iter() {
                push_f32(&mut buf, *v);
            }
            for v in normal.iter() {
                push_f32(&mut buf, *v);
            }
            for v in tangent.iter() {
                push_f32(&mut buf, *v);
            }
            for v in uv.iter() {
                push_f32(&mut buf, *v);
            }
        }
    }
    debug_assert_eq!(buf.len(), 48 * 24);
    buf
}

pub fn cube_index_buffer() -> Vec<u8> {
    let mut buf = Vec::with_capacity(36 * 4);
    for face in 0..6u32 {
        let base = face * 4;
        for tri in [[0u32, 1, 2], [0, 2, 3]].iter() {
            for offset in tri.iter() {
                push_u32(&mut buf, base + offset);
            }
        }
    }
    debug_assert_eq!(buf.len(), 36 * 4);
    buf
}

// =============================================================================
// Common UBO writers — these match the WGSL struct layouts in
// `crates/engine-render/shaders/`.
// =============================================================================

/// GBuffer's `PerFrame` UBO (gbuffer.wgsl:27). 224 B used, pad to 256
/// for std140 alignment. Source-of-truth columns: view-projection,
/// previous view-projection, view, jitter (xy current / zw prev),
/// camera position (w = 1).
pub fn gbuffer_perframe(view_projection: Mat4, view: Mat4, camera_pos: [f32; 3]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(256);
    push_mat4(&mut buf, view_projection);
    push_mat4(&mut buf, view_projection); // prev_view_projection = current (single-frame fixture)
    push_mat4(&mut buf, view);
    push_vec4(&mut buf, [0.0, 0.0, 0.0, 0.0]); // jitter
    push_vec4(&mut buf, [camera_pos[0], camera_pos[1], camera_pos[2], 1.0]);
    pad_to(&mut buf, 256);
    buf
}

/// Lighting's `FullScreenUniforms` UBO (lighting.wgsl:10). 96 B.
pub fn lighting_fullscreen(
    inv_view_projection: Mat4,
    camera_pos: [f32; 3],
    screen_extent: [u32; 2],
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(96);
    push_mat4(&mut buf, inv_view_projection);
    push_vec4(&mut buf, [camera_pos[0], camera_pos[1], camera_pos[2], 1.0]);
    push_f32(&mut buf, screen_extent[0] as f32);
    push_f32(&mut buf, screen_extent[1] as f32);
    push_f32(&mut buf, 0.0);
    push_f32(&mut buf, 0.0);
    debug_assert_eq!(buf.len(), 96);
    buf
}

/// Cluster `ClusterUniforms` UBO (cluster_assign.wgsl / lighting.wgsl).
/// 112 B with explicit padding per WGSL §13.4 (vec3<u32> alignment 16).
pub fn cluster_uniforms(
    inv_view_projection: Mat4,
    light_count: u32,
    grid_dim: [u32; 3],
    z_near: f32,
    z_far: f32,
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(112);
    push_mat4(&mut buf, inv_view_projection); //                   offset   0..63
    push_u32(&mut buf, light_count); //                            offset  64..67
    pad_to(&mut buf, 80); // pad for vec3<u32> 16-byte alignment   offset  68..79
    push_u32(&mut buf, grid_dim[0]); // grid_dim.x                 offset  80..83
    push_u32(&mut buf, grid_dim[1]); // grid_dim.y                 offset  84..87
    push_u32(&mut buf, grid_dim[2]); // grid_dim.z                 offset  88..91
    push_f32(&mut buf, z_near); //                                 offset  92..95
    push_f32(&mut buf, z_far); //                                  offset  96..99
    pad_to(&mut buf, 104); // pad for vec2<f32> 8-byte alignment   offset 100..103
    push_f32(&mut buf, 0.0); // reserved.x                         offset 104..107
    push_f32(&mut buf, 0.0); // reserved.y                         offset 108..111
    debug_assert_eq!(buf.len(), 112);
    buf
}

/// 6 frustum planes from the camera's view-projection matrix (96 B).
pub fn frustum_uniform(view_projection: Mat4) -> Vec<u8> {
    let frustum = engine_raster::Frustum::from_view_projection(view_projection);
    let mut buf = Vec::with_capacity(96);
    for plane in frustum.planes.iter() {
        push_f32(&mut buf, plane.normal.x);
        push_f32(&mut buf, plane.normal.y);
        push_f32(&mut buf, plane.normal.z);
        push_f32(&mut buf, plane.d);
    }
    debug_assert_eq!(buf.len(), 96);
    buf
}

/// Zero the draw-count atomic (16 B). CullPass uses `atomicAdd` to
/// claim slots in RID_INDIRECT.
pub fn zero_draw_count() -> Vec<u8> {
    vec![0u8; 16]
}

/// One `InstanceEntry` (cull.wgsl:17) — 32 B AABB + 4 × u32 = 48 B.
pub fn cull_instance_entry(aabb_min: [f32; 3], aabb_max: [f32; 3]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(48);
    push_f32(&mut buf, aabb_min[0]);
    push_f32(&mut buf, aabb_min[1]);
    push_f32(&mut buf, aabb_min[2]);
    push_f32(&mut buf, 0.0); // pad0
    push_f32(&mut buf, aabb_max[0]);
    push_f32(&mut buf, aabb_max[1]);
    push_f32(&mut buf, aabb_max[2]);
    push_f32(&mut buf, 0.0); // pad1
    push_u32(&mut buf, 0); // mesh_index
    push_u32(&mut buf, 0); // material_index
    push_u32(&mut buf, 0); // instance_id
    push_u32(&mut buf, 0); // flags
    debug_assert_eq!(buf.len(), 48);
    buf
}

/// One `MeshEntry` (cull.wgsl:34) — 4 × u32 = 16 B. Cube = 36 indices.
pub fn cull_mesh_entry(index_count: u32) -> Vec<u8> {
    let mut buf = Vec::with_capacity(16);
    push_u32(&mut buf, index_count);
    push_u32(&mut buf, 0); // first_index
    push_u32(&mut buf, 0); // base_vertex
    push_u32(&mut buf, 0); // flags
    debug_assert_eq!(buf.len(), 16);
    buf
}

/// One `InstanceDraw` (gbuffer.wgsl:17) — 64 B. `material_index` is the
/// bit-packed albedo (low 24 bits); `reserved` is the bit-packed
/// (roughness, metallic) auxiliary (low 16 bits).
///
/// Both fields are placeholders for the ADR-044 bindless per-material
/// SSBO. The encoding is documented in `cube.rs`'s
/// `instance_draw_for_cube` doc-comment.
pub fn instance_draw(albedo: [f32; 3], roughness: f32, metallic: f32, instance_id: u32) -> Vec<u8> {
    fn channel_byte(linear: f32) -> u32 {
        (linear.clamp(0.0, 1.0) * 255.0 + 0.5) as u32
    }
    let r = channel_byte(albedo[0]);
    let g = channel_byte(albedo[1]);
    let b = channel_byte(albedo[2]);
    let material_index = r | (g << 8) | (b << 16);
    let material_aux = channel_byte(roughness) | (channel_byte(metallic) << 8);

    let mut buf = Vec::with_capacity(64);
    push_vec4(&mut buf, [1.0, 0.0, 0.0, 0.0]); // x-axis | tx
    push_vec4(&mut buf, [0.0, 1.0, 0.0, 0.0]); // y-axis | ty
    push_vec4(&mut buf, [0.0, 0.0, 1.0, 0.0]); // z-axis | tz
    push_u32(&mut buf, material_index);
    push_u32(&mut buf, instance_id);
    push_u32(&mut buf, 0); // flags
    push_u32(&mut buf, material_aux);
    debug_assert_eq!(buf.len(), 64);
    buf
}

/// One `LightRecord` (lighting.wgsl:17). 64 B. The light-type tag in
/// `params.w` selects the kind: directional (`r ≤ 0`, params.w = 2),
/// point (`r > 0`, params.w = 0).
pub fn light_record_directional(direction: [f32; 3], color: [f32; 3], intensity: f32) -> Vec<u8> {
    let mut buf = Vec::with_capacity(64);
    push_vec4(&mut buf, [0.0, 0.0, 0.0, 0.0]); // position_radius (r = 0 → directional)
    push_vec4(&mut buf, [color[0], color[1], color[2], intensity]);
    push_vec4(&mut buf, [direction[0], direction[1], direction[2], 0.0]);
    push_vec4(&mut buf, [1.0, 0.0, 1.0, 2.0]); // params: light type = 2 (directional)
    debug_assert_eq!(buf.len(), 64);
    buf
}

/// One point light. `range` becomes the BRDF cluster-cell radius.
pub fn light_record_point(
    position: [f32; 3],
    color: [f32; 3],
    intensity: f32,
    range: f32,
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(64);
    push_vec4(&mut buf, [position[0], position[1], position[2], range]);
    push_vec4(&mut buf, [color[0], color[1], color[2], intensity]);
    push_vec4(&mut buf, [0.0, 0.0, 0.0, 0.0]); // direction unused for point
    push_vec4(&mut buf, [1.0, 0.0, 1.0, 0.0]); // params: light type = 0 (point)
    debug_assert_eq!(buf.len(), 64);
    buf
}

/// Tonemap UBO (16 B). Exposure / bloom_mix / white_point / reserved.
/// Post-Slice-8 the GPU shader ignores `white_point`.
pub fn tonemap_uniforms(exposure: f32, bloom_mix: f32) -> Vec<u8> {
    let mut buf = Vec::with_capacity(16);
    push_f32(&mut buf, exposure);
    push_f32(&mut buf, bloom_mix);
    push_f32(&mut buf, 1.0); // white_point (unused post-Slice-8)
    push_f32(&mut buf, 0.0); // reserved
    buf
}

/// TAA UBO (96 B). 16-byte aligned per WGSL §13.4.
pub fn taa_uniforms(prev_view_projection: Mat4, blend_alpha: f32) -> Vec<u8> {
    let mut buf = Vec::with_capacity(96);
    push_mat4(&mut buf, prev_view_projection);
    push_f32(&mut buf, 0.0); // jitter_current.x
    push_f32(&mut buf, 0.0); // jitter_current.y
    push_f32(&mut buf, 0.0); // jitter_prev.x
    push_f32(&mut buf, 0.0); // jitter_prev.y
    push_f32(&mut buf, blend_alpha);
    push_f32(&mut buf, 1.0); // disocclusion_threshold
    push_f32(&mut buf, 0.0); // reserved
    push_f32(&mut buf, 0.0); // reserved
    debug_assert_eq!(buf.len(), 96);
    buf
}

/// Bloom UBO (16 B).
pub fn bloom_uniforms(threshold: f32, soft_knee: f32, intensity: f32) -> Vec<u8> {
    let mut buf = Vec::with_capacity(16);
    push_f32(&mut buf, threshold);
    push_f32(&mut buf, soft_knee);
    push_f32(&mut buf, intensity);
    push_f32(&mut buf, 0.0);
    buf
}

/// CSM UBO — 4 cascade view-projection matrices = 4 × 64 = 256 B,
/// followed by per-cascade atlas origin + size (4 × vec4 = 64 B) =
/// 320 B; round to 384 B per harness allocation.
pub fn csm_uniforms(cascade_view_projections: [Mat4; 4]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(384);
    for vp in cascade_view_projections.iter() {
        push_mat4(&mut buf, *vp);
    }
    // 4 atlas-tile UVs (atlas_x_uv, atlas_y_uv, tile_size_uv, _unused).
    // The atlas is fixed at 4096² with 2048²-sized quadrants per
    // `engine_raster::shadow::atlas_origin`.
    let tile_uv_size = 0.5_f32;
    let origins = [
        [0.0_f32, 0.5_f32],
        [0.5_f32, 0.5_f32],
        [0.0_f32, 0.0_f32],
        [0.5_f32, 0.0_f32],
    ];
    for o in origins.iter() {
        push_vec4(&mut buf, [o[0], o[1], tile_uv_size, 0.0]);
    }
    pad_to(&mut buf, 384);
    buf
}

/// IBL UBO. The WGSL struct is 80 B (mat4 + u32 + f32 + vec2<f32>);
/// the harness allocates 96 B and the trailing 16 B are zero padding.
pub fn ibl_uniforms(inv_view_projection: Mat4, probe_count: u32, cell_size_m: f32) -> Vec<u8> {
    let mut buf = Vec::with_capacity(96);
    push_mat4(&mut buf, inv_view_projection);
    push_u32(&mut buf, probe_count);
    push_f32(&mut buf, cell_size_m);
    push_f32(&mut buf, 0.0); // reserved.x
    push_f32(&mut buf, 0.0); // reserved.y
    pad_to(&mut buf, 96);
    buf
}

/// SSAO UBO (256 B per the harness allocation; the shader's effective
/// UBO is smaller). Fields per shaders/ssao.wgsl:
/// inv_view_projection (64) + camera_pos (16) + screen_extent (8) +
/// pad (8) + radius (4) + bias (4) + intensity (4) + pad (4) = 112 B.
pub fn ssao_uniforms(
    inv_view_projection: Mat4,
    camera_pos: [f32; 3],
    screen_extent: [u32; 2],
    radius: f32,
    intensity: f32,
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(256);
    push_mat4(&mut buf, inv_view_projection);
    push_vec4(&mut buf, [camera_pos[0], camera_pos[1], camera_pos[2], 1.0]);
    push_f32(&mut buf, screen_extent[0] as f32);
    push_f32(&mut buf, screen_extent[1] as f32);
    push_f32(&mut buf, 0.0); // pad0
    push_f32(&mut buf, 0.0); // pad1
    push_f32(&mut buf, radius);
    push_f32(&mut buf, 0.05); // bias
    push_f32(&mut buf, intensity);
    push_f32(&mut buf, 0.0); // reserved
    pad_to(&mut buf, 256);
    buf
}

/// One IBL probe record (160 B per `IblProbeRecord` in
/// ibl_evaluate.wgsl): cell_key (vec3<i32> + pad) + 9 × vec4<f32> SH.
pub fn ibl_probe_record(cell_key: [i32; 3], sh_coeffs: [[f32; 3]; 9]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(160);
    push_i32(&mut buf, cell_key[0]);
    push_i32(&mut buf, cell_key[1]);
    push_i32(&mut buf, cell_key[2]);
    push_u32(&mut buf, 0); // pad
    for sh in sh_coeffs.iter() {
        push_vec4(&mut buf, [sh[0], sh[1], sh[2], 0.0]);
    }
    debug_assert_eq!(buf.len(), 160);
    buf
}
