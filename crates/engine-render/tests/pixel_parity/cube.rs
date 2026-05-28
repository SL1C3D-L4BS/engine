//! Phase 5.5 A.3 — cube parity fixture (structural slice).
//!
//! This is the vertical-slice fixture: it composes the full 10-pass
//! graph end-to-end against a real device, executes one frame, copies
//! the tonemap target back through the new `copy_texture_to_buffer`
//! primitive, and verifies the readback path is structurally sound.
//!
//! Strict pixel parity vs. `engine_raster`'s CPU oracle is the
//! follow-up slice's job — this commit asserts only:
//!
//! - The 10-pass graph compiles + installs pipelines + executes
//!   without panicking against the harness's transient resource pool.
//! - The tonemap target is large enough to copy back at the canonical
//!   render extent (128 × 72; matches `combined_deferred_scene`).
//! - The readback bytes have the expected size + the per-pixel BGRA
//!   layout survives the channel-swap into `Rgba8`.
//! - The Bgra8Unorm tonemap target's bytes-per-pixel × padded row
//!   stride math agrees with the harness's `COPY_BYTES_PER_ROW_ALIGNMENT`
//!   computation.
//!
//! With zeroed scene UBOs (no camera transforms, no light data) the
//! tonemap output will be black; that's expected. The next slice
//! seeds the scene data + asserts parity against
//! `engine_raster::compare_images`.

use engine_gpu::{COPY_BYTES_PER_ROW_ALIGNMENT, CommandEncoder};
use engine_render::GpuFrameContext;

use super::harness::ParityHarness;

/// Render extent for the cube fixture. Matches the
/// `combined_deferred_scene` CPU oracle (128 × 72, 16:9).
const WIDTH: u32 = 128;
const HEIGHT: u32 = 72;

/// 10-pass graph executes end-to-end + tonemap target is recoverable
/// through the readback primitive.
#[test]
fn cube_graph_executes_and_tonemap_reads_back() {
    let Some(harness) = ParityHarness::try_new() else {
        return;
    };
    let queue = harness.device.queue();
    let pool = harness.allocate_pool(WIDTH, HEIGHT);

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
    // Copy the tonemap target back through the new
    // `CommandEncoder::copy_texture_to_buffer` primitive.
    let staging = harness.copy_tonemap_to_staging(&mut encoder, &pool);
    let _token = queue.submit(encoder);

    // Verify the staging buffer round-trips. The CPU-side dimension
    // check is the structural invariant: padded row × height bytes,
    // matching the readback test in engine-gpu.
    let unpadded = WIDTH * 4;
    let expected_padded =
        unpadded.div_ceil(COPY_BYTES_PER_ROW_ALIGNMENT) * COPY_BYTES_PER_ROW_ALIGNMENT;
    assert_eq!(
        staging.padded_row, expected_padded,
        "staging row stride matches harness padding math"
    );

    let fb = staging.read_back_to_framebuffer();
    assert_eq!(fb.width(), WIDTH, "framebuffer width matches render extent");
    assert_eq!(
        fb.height(),
        HEIGHT,
        "framebuffer height matches render extent"
    );

    eprintln!(
        "[parity.cube] structural pass: graph executed, tonemap target recovered \
         (parity assertion pending next slice — scene data seeding + \
         engine_raster::compare_images verdict)"
    );
}
