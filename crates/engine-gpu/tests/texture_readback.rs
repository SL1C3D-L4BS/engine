//! Phase 5.5 A.3 prerequisite: texture-to-buffer readback round-trip.
//!
//! Writes a known per-pixel pattern into a 4×4 `Rgba8Unorm` texture via
//! [`Queue::write_texture_2d`], copies it back through
//! [`CommandEncoder::copy_texture_to_buffer`] + a MAP_READ staging buffer,
//! reads the staging buffer with [`Buffer::read_back`], and asserts every
//! pixel survived the round-trip. Proves the pixel-parity harness's
//! "render → readback → compare" path is structurally sound *before* it
//! gets composed against a 10-pass graph.
//!
//! Skips gracefully on hosts without a Vulkan loader (mirroring
//! [`pass_record_e2e`] in `engine-render`).

use std::panic::{self, AssertUnwindSafe};

use engine_gpu::{
    Buffer, BufferDesc, BufferUsage, COPY_BYTES_PER_ROW_ALIGNMENT, CommandEncoder, Device,
    DeviceLimits, Extent3d, Texture, TextureDesc, TextureDimension, TextureFormat, TextureUsage,
};

fn try_device() -> Option<Device> {
    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        Device::new(DeviceLimits::Tier1Minimum, false)
    }));
    match result {
        Ok(Ok(d)) => Some(d),
        Ok(Err(e)) => {
            eprintln!("[texture_readback] no compatible adapter (skipping): {e}");
            None
        }
        Err(_) => {
            eprintln!("[texture_readback] wgpu has no backend enabled (skipping)");
            None
        }
    }
}

/// Write a deterministic 4×4 RGBA pattern, read it back through the
/// encoder + staging buffer, assert byte-exact survival.
#[test]
fn rgba8_round_trip_via_copy_texture_to_buffer() {
    let Some(device) = try_device() else {
        return;
    };
    let queue = device.queue();

    const WIDTH: u32 = 4;
    const HEIGHT: u32 = 4;
    const BYTES_PER_PIXEL: u32 = 4;
    let padded_row = COPY_BYTES_PER_ROW_ALIGNMENT;
    let staging_size = padded_row as u64 * HEIGHT as u64;

    let texture = Texture::new(
        &device,
        &TextureDesc {
            label: "readback.src",
            extent: Extent3d::new_2d(WIDTH, HEIGHT),
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            usage: TextureUsage::COPY_SRC | TextureUsage::COPY_DST | TextureUsage::TEXTURE_BINDING,
        },
    );

    // Build the upload payload at the padded row stride. Each pixel
    // encodes its (x, y) — picking distinct channels per axis means a
    // swap or stride bug produces a visibly wrong byte rather than a
    // self-symmetric pass.
    let mut upload = vec![0u8; staging_size as usize];
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let row_base = (y * padded_row) as usize;
            let pix_base = row_base + (x * BYTES_PER_PIXEL) as usize;
            let idx = (y * WIDTH + x) as u8;
            upload[pix_base] = idx * 16; // r: rises with pixel index
            upload[pix_base + 1] = x as u8 * 64; // g: rises with x
            upload[pix_base + 2] = y as u8 * 64; // b: rises with y
            upload[pix_base + 3] = 255; // a: full
        }
    }
    queue.write_texture_2d(&texture, &upload, padded_row, HEIGHT);

    let staging = Buffer::new(
        &device,
        &BufferDesc {
            label: "readback.staging",
            size: staging_size,
            usage: BufferUsage::COPY_DST | BufferUsage::MAP_READ,
        },
    );

    let mut encoder = CommandEncoder::new(&device, "readback.encoder");
    encoder.copy_texture_to_buffer(&texture, &staging, padded_row, HEIGHT);
    let _token = queue.submit(encoder);

    let bytes = staging.read_back().expect("staging buffer maps for read");
    assert_eq!(
        bytes.len(),
        staging_size as usize,
        "readback returned wrong size"
    );

    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let row_base = (y * padded_row) as usize;
            let pix_base = row_base + (x * BYTES_PER_PIXEL) as usize;
            let idx = (y * WIDTH + x) as u8;
            assert_eq!(
                bytes[pix_base],
                idx * 16,
                "r channel mismatch at ({x}, {y})"
            );
            assert_eq!(
                bytes[pix_base + 1],
                x as u8 * 64,
                "g channel mismatch at ({x}, {y})"
            );
            assert_eq!(
                bytes[pix_base + 2],
                y as u8 * 64,
                "b channel mismatch at ({x}, {y})"
            );
            assert_eq!(bytes[pix_base + 3], 255, "a channel mismatch at ({x}, {y})");
        }
    }
}
