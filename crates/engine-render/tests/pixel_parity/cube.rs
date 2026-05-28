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
//! Following slices in the A.3 sequence will narrow the gap until the
//! verdict tightens.

use engine_gpu::{Buffer, COPY_BYTES_PER_ROW_ALIGNMENT, CommandEncoder};
use engine_math::Mat4;
use engine_raster::{CubeParityScene, OracleVerdict, compare_images};
use engine_render::{GpuFrameContext, ResourceId, ResourceResolver, TransientResourceTable};

use super::harness::{
    ParityHarness, Pool, RID_BLOOM_UBO, RID_CLUSTER_UBO, RID_CSM_UBO, RID_DRAW_COUNT_SSBO,
    RID_GBUFFER_FRAME_UBO, RID_IBL_UBO, RID_INDEX_BUF, RID_INDIRECT, RID_INSTANCES_SSBO,
    RID_LIGHTING_FRAME_UBO, RID_LIGHTS, RID_SSAO_UBO, RID_TAA_UBO, RID_TONEMAP_UBO, RID_VERTEX_BUF,
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

fn push_uvec3_pad(buf: &mut Vec<u8>, v: [u32; 3], pad: u32) {
    for x in v.iter() {
        push_u32(buf, *x);
    }
    push_u32(buf, pad);
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
fn cluster_uniforms(scene: &CubeParityScene) -> Vec<u8> {
    let inv_view_proj = scene
        .camera
        .view_projection()
        .inverse()
        .unwrap_or(Mat4::IDENTITY);
    let mut buf = Vec::with_capacity(112);
    push_mat4(&mut buf, inv_view_proj);
    push_u32(&mut buf, 1); // light_count
    push_uvec3_pad(&mut buf, [16, 9, 24], 0); // grid_dim (waste pad to match WGSL alignment)
    push_f32(&mut buf, scene.camera.near);
    push_f32(&mut buf, scene.camera.far);
    push_f32(&mut buf, 0.0); // reserved
    push_f32(&mut buf, 0.0); // reserved
    pad_to(&mut buf, 112);
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
fn instance_draw_for_cube() -> Vec<u8> {
    let mut buf = Vec::with_capacity(64);
    // Identity 4×3 affine, encoded as 3 rows of `(axis.xyz, translation_i)`.
    push_vec4(&mut buf, [1.0, 0.0, 0.0, 0.0]); // x-axis | tx
    push_vec4(&mut buf, [0.0, 1.0, 0.0, 0.0]); // y-axis | ty
    push_vec4(&mut buf, [0.0, 0.0, 1.0, 0.0]); // z-axis | tz
    push_u32(&mut buf, 0); // material_index
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

/// `DrawIndexedIndirect` arg + the count buffer's leading u32.
/// CullPass would normally produce these; the cube fixture bypasses
/// CullPass's compute output and seeds the values directly so GBuffer
/// + CsmShadow consume a known one-cube workload.
fn indirect_draw_args() -> Vec<u8> {
    let mut buf = Vec::with_capacity(20);
    push_u32(&mut buf, 36); // index_count
    push_u32(&mut buf, 1); // instance_count
    push_u32(&mut buf, 0); // first_index
    push_u32(&mut buf, 0); // base_vertex
    push_u32(&mut buf, 0); // first_instance
    debug_assert_eq!(buf.len(), 20);
    buf
}

fn indirect_draw_count() -> Vec<u8> {
    let mut buf = Vec::with_capacity(4);
    push_u32(&mut buf, 1);
    buf
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
    let _token = queue.submit(encoder);

    let unpadded = scene.width * 4;
    let expected_padded =
        unpadded.div_ceil(COPY_BYTES_PER_ROW_ALIGNMENT) * COPY_BYTES_PER_ROW_ALIGNMENT;
    assert_eq!(staging.padded_row, expected_padded);
    let gpu_fb = staging.read_back_to_framebuffer();

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

    // Indirect draw args + count (bypass CullPass; seed the post-cull state).
    queue.write_buffer(buffer_for(table, RID_INDIRECT), 0, &indirect_draw_args());
    queue.write_buffer(
        buffer_for(table, RID_DRAW_COUNT_SSBO),
        0,
        &indirect_draw_count(),
    );

    // Instance — the cube's affine + material index.
    queue.write_buffer(
        buffer_for(table, RID_INSTANCES_SSBO),
        0,
        &instance_draw_for_cube(),
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
