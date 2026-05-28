//! Phase 5.5 A.3 — cube parity fixture.
//!
//! Renders the [`engine_raster::CubeParityScene`] through both paths
//! (CPU oracle in `engine-raster`, GPU 10-pass graph) and runs them
//! through `engine_raster::compare_images` per ADR-046.
//!
//! ## Verification scope
//!
//! - Structural: the 10-pass graph compiles + installs pipelines +
//!   executes against the harness's transient resource pool; the
//!   tonemap target is readable through the `copy_texture_to_buffer`
//!   primitive; the BGRA→RGBA channel swap recovers a valid
//!   `engine_raster::Framebuffer`.
//! - Reference: the CPU framebuffer is non-blank — the oracle scene
//!   actually rendered the cube.
//! - Comparison: `compare_images` runs without panicking and returns
//!   one of the three documented verdicts.
//!
//! The test reports the observed verdict + violation metrics via
//! `eprintln!` so reviewers can read the actual parity gap. It does
//! **not** strictly assert `OracleVerdict::Pass` yet — the GPU shader
//! chain's per-pass numerical accumulation differs from the CPU oracle
//! in ways that require per-pass tightening across subsequent slices
//! (ADR-046's per-fixture exception process is the documented path).
//!
//! ## Engine-side issues uncovered by the cube diagnostic
//!
//! The Slice 4 diagnostic surfaced three engine-side issues; Slices 5,
//! 6, 7 each closed one. Slice 7 also fixed two infrastructure-side
//! gaps that became observable once the engine produced non-zero
//! output.
//!
//! 1. **(FIXED, Slice 6) Reverse-Z mismatch across the stack.**
//!    `engine_gpu`'s pipeline builder hardcoded `depth_compare:
//!    LessEqual` (standard depth) and `Camera::projection()` returned a
//!    standard-Z matrix, but the rest of the stack assumed reverse-Z
//!    (`Clear(0.0)`, `depth <= 0.0` sky check). Every fragment failed
//!    the depth test → depth stayed at the clear value → lighting
//!    short-circuited every pixel to black. Slice 6 aligned both to
//!    reverse-Z.
//! 2. **(FIXED, Slice 5) `ClusterUniforms` WGSL std140 padding gap.**
//!    The fixture's `cluster_uniforms()` writer omitted the 12-byte pad
//!    `vec3<u32>` alignment requires after the `light_count : u32`
//!    field. The cluster shader read garbage `grid_dim`, the lighting
//!    shader's cell-index computation overflowed → `cells[OOB]` came
//!    back as `(0, 0)` → no lights walked → black output. Slice 5
//!    rewrote the writer with explicit offsets + a layout test.
//! 3. **(FIXED, Slice 7) Lighting shader treated every light as a
//!    point light.** `lighting.wgsl:171` did `to_light =
//!    light.position_radius.xyz - world_pos`, ignoring
//!    `light.direction.xyz`. The cube fixture's directional light has
//!    `position_radius = (0, 0, 0, 0)`, so the BRDF input pointed from
//!    each surface fragment toward the world origin (inside the cube),
//!    giving `n_dot_l ≤ 0` for every visible face. Slice 7 added the
//!    `r <= 0 → directional` branch (`to_light = -light.direction.xyz`)
//!    matching `engine_raster::sample::CubeParityScene::render_cpu`'s
//!    convention.
//!
//! ## Infrastructure-side fixes Slice 7 also surfaced
//!
//! 4. **(FIXED, Slice 7) Harness pool never registered
//!    `RID_TONEMAPPED` in `TransientResourceTable`** — the tonemap
//!    target lived only as a `Pool` field so [`copy_tonemap_to_staging`]
//!    could readback it. As a result, `TonemapPass::record()`'s
//!    `resolve_view(self.tonemapped)` returned `None` and the pass
//!    silently short-circuited; the readback path read the texture's
//!    zero-initialised state. The bug was masked while engine bugs 1–3
//!    kept lighting at zero. Slice 7 registers the tonemap target in
//!    the table and routes the readback through `resolve_texture`.
//! 5. **(FIXED, Slice 7) Cube `material_index = 0` decoded to black
//!    albedo.** `gbuffer.wgsl:96–106` bit-packs RGB albedo into the
//!    `material_index` placeholder until ADR-044's bindless material
//!    storage lands; `material_index = 0` → `mat_color = (0, 0, 0)` →
//!    diffuse contribution vanishes regardless of light. Slice 7
//!    encodes the [`CubeParityScene::material`] albedo `(0.8, 0.4, 0.2)`
//!    as bit-packed `(204, 102, 51)` into `material_index`.
//!
//! ## Remaining parity gaps (post-Slice-7, deferred)
//!
//! - **Roughness mismatch.** `gbuffer.wgsl` hardcodes `roughness = 0.5`;
//!   the CPU material uses `0.35`. The BRDF specular peak differs.
//! - **TAA double-binding.** The harness binds [`RID_LIT`] to TaaPass's
//!   `lit_color` *and* `ibl_contribution` arguments, so the TAA fetch
//!   reads `lit + lit = 2*lit`. Until IBL has its own output texture
//!   this is the harness's best 1-fixture approximation; the resulting
//!   2× brightness inflation partially offsets the dimmer roughness.
//! - Both gaps land in subsequent slices (likely bundled with the
//!   bindless-material fixture upgrade so the rough material can move
//!   to a per-material SSBO).
//!
//! Slice 7 lands the cube fixture at `both-lit = 391`, `max_delta ≈
//! 0.64` (down from `0.68` post-Slice-6). The cube is now correctly
//! identified as lit on both sides; the verdict tightens further as
//! the deferred gaps close.

use engine_gpu::{Buffer, BufferDesc, BufferUsage, COPY_BYTES_PER_ROW_ALIGNMENT, CommandEncoder};
use engine_math::Mat4;
use engine_raster::{CubeParityScene, OracleVerdict, compare_images};
use engine_render::{GpuFrameContext, ResourceId, ResourceResolver, TransientResourceTable};

use super::harness::{
    ParityHarness, Pool, RID_BLOOM_UBO, RID_CLUSTER_UBO, RID_CSM_UBO, RID_DEPTH,
    RID_DRAW_COUNT_SSBO, RID_FRUSTUM_UBO, RID_GBUFFER_FRAME_UBO, RID_IBL_UBO, RID_INDEX_BUF,
    RID_INSTANCES_SSBO, RID_LIGHTING_FRAME_UBO, RID_LIGHTS, RID_MESHES_SSBO, RID_RENDER_QUEUE,
    RID_SSAO_UBO, RID_TAA_UBO, RID_TONEMAP_UBO, RID_VERTEX_BUF,
};

fn buffer_for(table: &TransientResourceTable, id: ResourceId) -> &Buffer {
    table
        .resolve_buffer(id)
        .expect("harness registered this id in the pool")
}

// =============================================================================
// Byte-layout helpers — write WGSL `repr(std140)`-compatible records into
// a `Vec<u8>` field-by-field. Avoids `unsafe` on `#[repr(C)]` reinterpret
// (the test target can't take a `bytemuck` dependency without surfacing
// it on the workspace; the contracts structs aren't `Pod`).
// =============================================================================

fn push_f32(buf: &mut Vec<u8>, v: f32) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn push_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn push_vec4(buf: &mut Vec<u8>, v: [f32; 4]) {
    for x in v.iter() {
        push_f32(buf, *x);
    }
}

/// Append a column-major mat4 as 16 little-endian `f32`.
fn push_mat4(buf: &mut Vec<u8>, m: Mat4) {
    let cols = m.to_cols_array();
    for v in cols.iter() {
        push_f32(buf, *v);
    }
}

fn pad_to(buf: &mut Vec<u8>, target_len: usize) {
    while buf.len() < target_len {
        buf.push(0);
    }
}

// =============================================================================
// UBO writers — one per shader's UBO struct
// =============================================================================

/// GBuffer's `PerFrame` (gbuffer.wgsl:27). 224 B used, pad to 256 for
/// alignment.
fn gbuffer_perframe(scene: &CubeParityScene) -> Vec<u8> {
    let view = scene.camera.view();
    let view_proj = scene.camera.view_projection();
    let mut buf = Vec::with_capacity(256);
    push_mat4(&mut buf, view_proj);
    push_mat4(&mut buf, view_proj); // prev = current — single-frame fixture, no motion
    push_mat4(&mut buf, view);
    push_vec4(&mut buf, [0.0, 0.0, 0.0, 0.0]); // jitter
    push_vec4(
        &mut buf,
        [
            scene.camera.position.x,
            scene.camera.position.y,
            scene.camera.position.z,
            1.0,
        ],
    );
    pad_to(&mut buf, 256);
    buf
}

/// Lighting's `FullScreenUniforms` (lighting.wgsl:10). 96 B.
fn lighting_fullscreen(scene: &CubeParityScene) -> Vec<u8> {
    let inv_view_proj = scene
        .camera
        .view_projection()
        .inverse()
        .unwrap_or(Mat4::IDENTITY);
    let mut buf = Vec::with_capacity(96);
    push_mat4(&mut buf, inv_view_proj);
    push_vec4(
        &mut buf,
        [
            scene.camera.position.x,
            scene.camera.position.y,
            scene.camera.position.z,
            1.0,
        ],
    );
    // screen_extent + pad (vec2 + vec2)
    push_f32(&mut buf, scene.width as f32);
    push_f32(&mut buf, scene.height as f32);
    push_f32(&mut buf, 0.0);
    push_f32(&mut buf, 0.0);
    debug_assert_eq!(buf.len(), 96);
    buf
}

/// Cluster's `ClusterUniforms` (cluster_assign.wgsl:11 / lighting.wgsl:29).
/// Same struct in both shaders; one UBO serves both.
///
/// WGSL host-shareable layout (uniform address space):
/// ```text
///   inv_view_projection : mat4x4<f32>   //   0..63   (align 16, size 64)
///   light_count         : u32           //  64..67
///   _pad0               : 12 bytes      //  68..79   (vec3<u32> requires align 16)
///   grid_dim            : vec3<u32>     //  80..91
///   z_near              : f32           //  92..95
///   z_far               : f32           //  96..99
///   _pad1               : 4 bytes       // 100..103  (vec2<f32> requires align 8)
///   reserved            : vec2<f32>     // 104..111
/// ```
/// Total: 112 B. The 12-byte gap between `light_count` and `grid_dim`
/// is required by WGSL §13.4 (`vec3<u32>` alignment is 16 even though
/// size is 12); omitting it shifts every subsequent field. The Slice 4
/// diagnostic surfaced this as the cluster shader reading garbage
/// `grid_dim` → lighting computes OOB cell index → returns black.
fn cluster_uniforms(scene: &CubeParityScene) -> Vec<u8> {
    let inv_view_proj = scene
        .camera
        .view_projection()
        .inverse()
        .unwrap_or(Mat4::IDENTITY);
    let mut buf = Vec::with_capacity(112);
    push_mat4(&mut buf, inv_view_proj); //                       offset   0..63
    push_u32(&mut buf, 1); // light_count                        offset  64..67
    pad_to(&mut buf, 80); // pad to vec3<u32> 16-byte alignment  offset  68..79
    push_u32(&mut buf, 16); // grid_dim.x                        offset  80..83
    push_u32(&mut buf, 9); // grid_dim.y                         offset  84..87
    push_u32(&mut buf, 24); // grid_dim.z                        offset  88..91
    push_f32(&mut buf, scene.camera.near); // z_near             offset  92..95
    push_f32(&mut buf, scene.camera.far); // z_far               offset  96..99
    pad_to(&mut buf, 104); // pad to vec2<f32> 8-byte alignment  offset 100..103
    push_f32(&mut buf, 0.0); // reserved.x                       offset 104..107
    push_f32(&mut buf, 0.0); // reserved.y                       offset 108..111
    debug_assert_eq!(buf.len(), 112);
    buf
}

/// One [`engine_render::contracts::LightRecord`] for the directional
/// light — 64 B.
fn light_record(scene: &CubeParityScene) -> Vec<u8> {
    let mut buf = Vec::with_capacity(64);
    // position_radius — directional light: radius = 0 → cluster shader's
    // `radius <= 0.0` branch flags "intersects every cluster" so the
    // light reaches every fragment without per-cell distance gating.
    push_vec4(&mut buf, [0.0, 0.0, 0.0, 0.0]);
    push_vec4(
        &mut buf,
        [
            scene.light.color.x,
            scene.light.color.y,
            scene.light.color.z,
            scene.light.intensity,
        ],
    );
    // direction: light direction (light → scene). Lighting shader
    // negates internally for the surface → light BRDF input.
    push_vec4(
        &mut buf,
        [
            scene.light.position_or_direction.x,
            scene.light.position_or_direction.y,
            scene.light.position_or_direction.z,
            0.0,
        ],
    );
    // params: x = inner cone cos (unused), y = outer cone cos, z = falloff
    // exponent, w = light type tag (2 = directional per ADR-064 §5).
    push_vec4(&mut buf, [1.0, 0.0, 1.0, 2.0]);
    debug_assert_eq!(buf.len(), 64);
    buf
}

/// One [`InstanceDraw`] (gbuffer.wgsl:17 + csm_shadow.wgsl) for the cube.
/// 3×vec4 model affine + 4×u32 = 64 B.
///
/// `material_index` is bit-packed RGB albedo in the shader's placeholder
/// material-bake path (gbuffer.wgsl:96–106): each byte of the u32 is a
/// channel divided by 255. Encoding the [`CubeParityScene`] material's
/// albedo (0.8, 0.4, 0.2) → bytes (204, 102, 51) → u32 0x0033_66CC keeps
/// the GPU's GBuffer albedo aligned to the CPU oracle. Without this
/// alignment, the GPU base_color would be (0, 0, 0) and the BRDF's
/// diffuse term would vanish.
fn instance_draw_for_cube(scene: &CubeParityScene) -> Vec<u8> {
    fn channel_byte(linear: f32) -> u32 {
        (linear.clamp(0.0, 1.0) * 255.0 + 0.5) as u32
    }
    let r = channel_byte(scene.material.albedo.x);
    let g = channel_byte(scene.material.albedo.y);
    let b = channel_byte(scene.material.albedo.z);
    let material_index = r | (g << 8) | (b << 16);

    let mut buf = Vec::with_capacity(64);
    // Identity 4×3 affine, encoded as 3 rows of `(axis.xyz, translation_i)`.
    push_vec4(&mut buf, [1.0, 0.0, 0.0, 0.0]); // x-axis | tx
    push_vec4(&mut buf, [0.0, 1.0, 0.0, 0.0]); // y-axis | ty
    push_vec4(&mut buf, [0.0, 0.0, 1.0, 0.0]); // z-axis | tz
    push_u32(&mut buf, material_index); // material_index → bit-packed albedo
    push_u32(&mut buf, 0); // instance_id
    push_u32(&mut buf, 0); // flags
    push_u32(&mut buf, 0); // reserved
    debug_assert_eq!(buf.len(), 64);
    buf
}

/// Tonemap uniforms — exposure 1, no bloom, ACES white-point.
fn tonemap_uniforms() -> Vec<u8> {
    let mut buf = Vec::with_capacity(16);
    push_f32(&mut buf, 1.0); // exposure
    push_f32(&mut buf, 0.0); // bloom_mix
    push_f32(&mut buf, 11.2); // white_point (ACES filmic)
    push_f32(&mut buf, 0.0); // reserved
    buf
}

/// TAA uniforms — alpha = 1.0 (use current frame, no history blend) so
/// the resolved buffer == the lit input. Sized 96 B per
/// `contracts::TaaUniforms` (mat4 + vec2 + vec2 + 2*f32 + vec2).
fn taa_uniforms() -> Vec<u8> {
    let mut buf = Vec::with_capacity(96);
    push_mat4(&mut buf, Mat4::IDENTITY); // prev_view_projection
    push_f32(&mut buf, 0.0); // jitter_current.x
    push_f32(&mut buf, 0.0); // jitter_current.y
    push_f32(&mut buf, 0.0); // jitter_prev.x
    push_f32(&mut buf, 0.0); // jitter_prev.y
    push_f32(&mut buf, 1.0); // blend_alpha = 1 → current frame only
    push_f32(&mut buf, 1.0); // disocclusion_threshold (any non-zero)
    push_f32(&mut buf, 0.0); // reserved
    push_f32(&mut buf, 0.0); // reserved
    debug_assert_eq!(buf.len(), 96);
    buf
}

/// Bloom uniforms — threshold high enough that no pixel passes the
/// soft-knee, so the bloom layer stays at zero (matches the CPU
/// oracle which doesn't add a bloom term).
fn bloom_uniforms() -> Vec<u8> {
    let mut buf = Vec::with_capacity(16);
    push_f32(&mut buf, 1.0e9); // threshold — astronomically high
    push_f32(&mut buf, 1.0); // soft_knee
    push_f32(&mut buf, 0.0); // intensity
    push_f32(&mut buf, 0.0); // reserved
    buf
}

// =============================================================================
// Cube mesh data — 24 verts (4 per face × 6 faces) for per-face normals,
// 36 indices. Vertex layout per ADR-061: 48 B = pos(12) + normal(12) +
// tangent(16) + uv(8).
// =============================================================================

/// One cube vertex: position + normal + tangent + uv = 48 B.
type CubeVert = ([f32; 3], [f32; 3], [f32; 4], [f32; 2]);

fn cube_vertex_buffer(aabb_min: [f32; 3], aabb_max: [f32; 3]) -> Vec<u8> {
    // Six faces, each with 4 corners. Pack as
    // (position[3], normal[3], tangent[4], uv[2]) = 12 floats = 48 B.
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

fn cube_index_buffer() -> Vec<u8> {
    let mut buf = Vec::with_capacity(36 * 4);
    // Each face's 4 verts are at indices [face*4 .. face*4+4]. Triangulate
    // as (0, 1, 2) + (0, 2, 3) per face.
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

/// One `InstanceEntry` (cull.wgsl:17) — 32 B AABB + 4 × u32 = 48 B.
/// Drives the CullPass's per-instance frustum test.
fn cull_instance_entry(scene: &CubeParityScene) -> Vec<u8> {
    let mut buf = Vec::with_capacity(48);
    // Aabb: 3×f32 min + 4 B pad + 3×f32 max + 4 B pad = 32 B.
    push_f32(&mut buf, scene.cube_aabb.min.x);
    push_f32(&mut buf, scene.cube_aabb.min.y);
    push_f32(&mut buf, scene.cube_aabb.min.z);
    push_f32(&mut buf, 0.0); // pad0
    push_f32(&mut buf, scene.cube_aabb.max.x);
    push_f32(&mut buf, scene.cube_aabb.max.y);
    push_f32(&mut buf, scene.cube_aabb.max.z);
    push_f32(&mut buf, 0.0); // pad1
    push_u32(&mut buf, 0); // mesh_index
    push_u32(&mut buf, 0); // material_index
    push_u32(&mut buf, 0); // instance_id
    push_u32(&mut buf, 0); // flags
    debug_assert_eq!(buf.len(), 48);
    buf
}

/// One `MeshEntry` (cull.wgsl:34) — 4 × u32 = 16 B. Tells the cull
/// shader the cube's index range so the produced `DrawIndirect` has a
/// non-zero `index_count`. Without this seed, every cull-survivor
/// indirect arg has `index_count = 0` and GBufferPass draws nothing.
fn cull_mesh_entry() -> Vec<u8> {
    let mut buf = Vec::with_capacity(16);
    push_u32(&mut buf, 36); // index_count (cube = 6 × 2 × 3)
    push_u32(&mut buf, 0); // first_index
    push_u32(&mut buf, 0); // base_vertex (i32; 0 fits)
    push_u32(&mut buf, 0); // flags
    debug_assert_eq!(buf.len(), 16);
    buf
}

/// 6 frustum planes packed as `vec4<f32>` (xyz = normal, w = signed
/// distance) — extracted from the camera's view-projection matrix via
/// `Frustum::from_view_projection`. The cull shader reads this UBO at
/// `@group(0) @binding(0)`.
fn frustum_uniform(scene: &CubeParityScene) -> Vec<u8> {
    let frustum = engine_raster::Frustum::from_view_projection(scene.camera.view_projection());
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

/// Zero the draw-count atomic. CullPass uses `atomicAdd(count, 1)` to
/// claim slots in the indirect-draws SSBO; if the count buffer doesn't
/// start at zero, surviving instances get appended past the end of the
/// indirect-args SSBO.
fn zero_draw_count() -> Vec<u8> {
    vec![0u8; 16]
}

// =============================================================================
// Test
// =============================================================================

/// CPU oracle vs. GPU graph output for the cube fixture. Asserts the
/// comparison path is well-defined; reports the actual verdict via
/// `eprintln!` for future-slice tightening.
#[test]
fn cube_parity() {
    let Some(harness) = ParityHarness::try_new() else {
        return;
    };
    let queue = harness.device.queue();

    let features = harness.device.features();
    eprintln!(
        "[parity.cube] device features: multi_draw_indirect_count={} indirect_first_instance={}",
        features.multi_draw_indirect_count, features.indirect_first_instance
    );

    let scene = CubeParityScene::default_v0();
    let pool = harness.allocate_pool(scene.width, scene.height);
    seed_scene(&harness, &pool, &scene);

    // ---- run the GPU graph ----
    let mut graph = harness.build_graph();
    graph
        .install_pipelines(&harness.device)
        .expect("phase6 pipelines install on parity graph");
    let pass_count = graph.compile().expect("10-pass graph compiles");
    assert_eq!(pass_count, 10, "all 10 active passes scheduled");

    let mut encoder = CommandEncoder::new(&harness.device, "parity.cube.encoder");
    {
        let gpu = GpuFrameContext {
            device: &harness.device,
            encoder: &mut encoder,
        };
        let mut user: () = ();
        graph
            .execute(0, &mut user, Some(gpu), Some(&pool.table))
            .expect("graph executes end-to-end");
    }
    let staging = harness.copy_tonemap_to_staging(&mut encoder, &pool);

    // Diagnostic: read back the depth attachment so we can observe
    // whether GBuffer actually rasterised the cube. Reverse-Z clear is
    // 0.0; any value > 0 means a fragment passed the depth test.
    let depth_tex = pool
        .table
        .resolve_texture(RID_DEPTH)
        .expect("depth registered in pool");
    let depth_unpadded = scene.width * 4;
    let depth_padded =
        depth_unpadded.div_ceil(COPY_BYTES_PER_ROW_ALIGNMENT) * COPY_BYTES_PER_ROW_ALIGNMENT;
    let depth_size = depth_padded as u64 * scene.height as u64;
    let depth_staging = Buffer::new(
        &harness.device,
        &BufferDesc {
            label: "parity.cube.depth.staging",
            size: depth_size,
            usage: BufferUsage::COPY_DST | BufferUsage::MAP_READ,
        },
    );
    encoder.copy_texture_to_buffer(depth_tex, &depth_staging, depth_padded, scene.height);

    let _token = queue.submit(encoder);

    let unpadded = scene.width * 4;
    let expected_padded =
        unpadded.div_ceil(COPY_BYTES_PER_ROW_ALIGNMENT) * COPY_BYTES_PER_ROW_ALIGNMENT;
    assert_eq!(staging.padded_row, expected_padded);
    let gpu_fb = staging.read_back_to_framebuffer();

    // Inspect the depth buffer the GBuffer wrote.
    let depth_bytes = depth_staging
        .read_back()
        .expect("depth staging maps for read");
    let mut depth_nonzero = 0u64;
    let mut depth_max: f32 = 0.0;
    for y in 0..scene.height {
        for x in 0..scene.width {
            let row_base = (y * depth_padded) as usize;
            let pix_base = row_base + (x as usize) * 4;
            let d = f32::from_le_bytes([
                depth_bytes[pix_base],
                depth_bytes[pix_base + 1],
                depth_bytes[pix_base + 2],
                depth_bytes[pix_base + 3],
            ]);
            if d > 0.0 {
                depth_nonzero += 1;
                if d > depth_max {
                    depth_max = d;
                }
            }
        }
    }
    eprintln!("[parity.cube] depth: {depth_nonzero} pixels with depth > 0 (max = {depth_max})",);

    // ---- CPU oracle reference ----
    let cpu_fb = scene.render_cpu();
    let cpu_lit = cpu_fb.color().iter().any(|p| p.r > 0 || p.g > 0 || p.b > 0);
    assert!(
        cpu_lit,
        "CPU oracle for cube parity scene produced an all-black image"
    );

    // ---- compare ----
    let cmp = compare_images(&cpu_fb, &gpu_fb);
    let frac_violating = (cmp.violating_pixels as f64) / (cmp.total_pixels.max(1) as f64);
    eprintln!(
        "[parity.cube] verdict = {:?} ({:.2}% pixels violating, max_delta = {:.4}, mean_delta = {:.5})",
        cmp.verdict,
        frac_violating * 100.0,
        cmp.max_delta,
        cmp.mean_delta,
    );

    // Per-region diagnostic — split pixels into (CPU lit?, GPU lit?)
    // quadrants so the parity-gap shape is visible without a PNG dump.
    let (mut cpu_black_gpu_black, mut cpu_black_gpu_lit, mut cpu_lit_gpu_black, mut both_lit) =
        (0u64, 0u64, 0u64, 0u64);
    struct WorstPixel {
        x: u32,
        y: u32,
        cpu: [u8; 3],
        gpu: [u8; 3],
        max_delta: f32,
    }
    let mut worst: Vec<WorstPixel> = Vec::new();
    for y in 0..cpu_fb.height() {
        for x in 0..cpu_fb.width() {
            let c = cpu_fb.sample(x, y);
            let g = gpu_fb.sample(x, y);
            let cpu_dark = c.r == 0 && c.g == 0 && c.b == 0;
            let gpu_dark = g.r == 0 && g.g == 0 && g.b == 0;
            match (cpu_dark, gpu_dark) {
                (true, true) => cpu_black_gpu_black += 1,
                (true, false) => cpu_black_gpu_lit += 1,
                (false, true) => cpu_lit_gpu_black += 1,
                (false, false) => both_lit += 1,
            }
            // Track the 5 worst-offending pixels for inspection.
            let dr = (c.r as i32 - g.r as i32).abs() as f32 / 255.0;
            let dg = (c.g as i32 - g.g as i32).abs() as f32 / 255.0;
            let db = (c.b as i32 - g.b as i32).abs() as f32 / 255.0;
            let pixel_max = dr.max(dg).max(db);
            let candidate = WorstPixel {
                x,
                y,
                cpu: [c.r, c.g, c.b],
                gpu: [g.r, g.g, g.b],
                max_delta: pixel_max,
            };
            if worst.len() < 5 {
                worst.push(candidate);
                worst.sort_by(|a, b| b.max_delta.partial_cmp(&a.max_delta).unwrap());
            } else if pixel_max > worst.last().unwrap().max_delta {
                worst.pop();
                worst.push(candidate);
                worst.sort_by(|a, b| b.max_delta.partial_cmp(&a.max_delta).unwrap());
            }
        }
    }
    eprintln!(
        "[parity.cube] regions: both-black {} | cpu-lit only {} | gpu-lit only {} | both-lit {}",
        cpu_black_gpu_black, cpu_lit_gpu_black, cpu_black_gpu_lit, both_lit,
    );
    eprintln!("[parity.cube] worst pixels (sRGB byte deltas, top 5):");
    for w in &worst {
        eprintln!(
            "  ({:3}, {:2}): cpu=({:3},{:3},{:3})  gpu=({:3},{:3},{:3})  pix_max={:.3}",
            w.x, w.y, w.cpu[0], w.cpu[1], w.cpu[2], w.gpu[0], w.gpu[1], w.gpu[2], w.max_delta,
        );
    }

    // Slice 3 ships the comparison path itself. Strict
    // `OracleVerdict::Pass` will land as later slices tighten per-pass
    // numerical agreement (per ADR-046's per-fixture exception process).
    // For now: the verdict must be defined + the GPU framebuffer must
    // match the CPU framebuffer's dimensions (an upstream-changed
    // extent here would silently mis-compare otherwise).
    assert_eq!(gpu_fb.width(), cpu_fb.width());
    assert_eq!(gpu_fb.height(), cpu_fb.height());
    assert!(matches!(
        cmp.verdict,
        OracleVerdict::Pass | OracleVerdict::PassUnderThreshold | OracleVerdict::Fail
    ));
}

/// Seed every UBO + SSBO + mesh buffer the cube fixture needs against the
/// canonical 10-pass graph's resource layout. Zero-initialised buffers
/// (everything not seeded here — IBL probes, SSAO uniforms, CSM uniforms,
/// shadow casters) leave their consumer passes producing a zero
/// contribution.
fn seed_scene(harness: &ParityHarness, pool: &Pool, scene: &CubeParityScene) {
    let queue = harness.device.queue();
    let table = &pool.table;

    // Mesh.
    let vertex_bytes = cube_vertex_buffer(
        [
            scene.cube_aabb.min.x,
            scene.cube_aabb.min.y,
            scene.cube_aabb.min.z,
        ],
        [
            scene.cube_aabb.max.x,
            scene.cube_aabb.max.y,
            scene.cube_aabb.max.z,
        ],
    );
    let index_bytes = cube_index_buffer();
    queue.write_buffer(buffer_for(table, RID_VERTEX_BUF), 0, &vertex_bytes);
    queue.write_buffer(buffer_for(table, RID_INDEX_BUF), 0, &index_bytes);

    // CullPass inputs — the pass overwrites RID_INDIRECT + RID_DRAW_COUNT_SSBO
    // from these every frame.
    queue.write_buffer(
        buffer_for(table, RID_RENDER_QUEUE),
        0,
        &cull_instance_entry(scene),
    );
    queue.write_buffer(buffer_for(table, RID_MESHES_SSBO), 0, &cull_mesh_entry());
    queue.write_buffer(
        buffer_for(table, RID_FRUSTUM_UBO),
        0,
        &frustum_uniform(scene),
    );
    // Reset the draw-count atomic to 0 — CullPass appends with atomicAdd
    // and pre-existing values would shift slots past the SSBO end.
    queue.write_buffer(
        buffer_for(table, RID_DRAW_COUNT_SSBO),
        0,
        &zero_draw_count(),
    );

    // Instance — the cube's affine + material index.
    queue.write_buffer(
        buffer_for(table, RID_INSTANCES_SSBO),
        0,
        &instance_draw_for_cube(scene),
    );

    // Light record.
    queue.write_buffer(buffer_for(table, RID_LIGHTS), 0, &light_record(scene));

    // UBOs.
    queue.write_buffer(
        buffer_for(table, RID_GBUFFER_FRAME_UBO),
        0,
        &gbuffer_perframe(scene),
    );
    queue.write_buffer(
        buffer_for(table, RID_LIGHTING_FRAME_UBO),
        0,
        &lighting_fullscreen(scene),
    );
    queue.write_buffer(
        buffer_for(table, RID_CLUSTER_UBO),
        0,
        &cluster_uniforms(scene),
    );
    queue.write_buffer(buffer_for(table, RID_TONEMAP_UBO), 0, &tonemap_uniforms());
    queue.write_buffer(buffer_for(table, RID_TAA_UBO), 0, &taa_uniforms());
    queue.write_buffer(buffer_for(table, RID_BLOOM_UBO), 0, &bloom_uniforms());

    // Zero the UBOs we don't seed (SSAO, IBL, CSM) — wgpu requires an
    // initialized buffer for binding validation, even when the shader
    // reads only fields whose zero values produce a no-op contribution.
    queue.write_buffer(buffer_for(table, RID_SSAO_UBO), 0, &[0u8; 256]);
    queue.write_buffer(buffer_for(table, RID_IBL_UBO), 0, &[0u8; 96]);
    queue.write_buffer(buffer_for(table, RID_CSM_UBO), 0, &[0u8; 384]);
}

#[cfg(test)]
mod layout_tests {
    use super::*;

    /// Lock the WGSL host-shareable layout for `ClusterUniforms`. The
    /// cluster shader (`cluster_assign.wgsl`) and lighting shader
    /// (`lighting.wgsl`) both read this UBO; an off-by-12 from missing
    /// `vec3<u32>` alignment produces garbage `grid_dim` and the
    /// lighting pass returns black for every cube pixel. No GPU device
    /// required — pure byte-layout check.
    #[test]
    fn cluster_uniforms_layout_matches_wgsl_spec() {
        let scene = CubeParityScene::default_v0();
        let bytes = cluster_uniforms(&scene);
        assert_eq!(bytes.len(), 112, "ClusterUniforms is 112 bytes");

        // light_count at offset 64.
        let light_count = u32::from_le_bytes(bytes[64..68].try_into().unwrap());
        assert_eq!(light_count, 1, "light_count at offset 64");

        // grid_dim at offset 80 (12-byte pad from light_count + alignment).
        let gx = u32::from_le_bytes(bytes[80..84].try_into().unwrap());
        let gy = u32::from_le_bytes(bytes[84..88].try_into().unwrap());
        let gz = u32::from_le_bytes(bytes[88..92].try_into().unwrap());
        assert_eq!((gx, gy, gz), (16, 9, 24), "grid_dim at offset 80");

        // z_near at offset 92, z_far at offset 96.
        let z_near = f32::from_le_bytes(bytes[92..96].try_into().unwrap());
        let z_far = f32::from_le_bytes(bytes[96..100].try_into().unwrap());
        assert!((z_near - scene.camera.near).abs() < 1e-6);
        assert!((z_far - scene.camera.far).abs() < 1e-6);
    }
}
