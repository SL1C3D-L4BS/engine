//! Phase 5.5 A.2b-ii init-time smoke tests.
//!
//! Proves the one-shot helpers in [`engine_render::init`] reach the
//! "real GPU" path against an actual adapter without panicking. The
//! ADR-075 §1 template's Step-3..6 (bind group + dispatch + submit)
//! exercise here against a real Polaris / RDNA / cross-vendor device,
//! catching any binding-format / dispatch-dimension drift between the
//! WGSL shader and the Rust-side helper.
//!
//! Skips gracefully on hosts without a Vulkan loader (same panic guard
//! ADR-074's `pipeline_smoke.rs` uses).

use std::panic::{self, AssertUnwindSafe};

use engine_gpu::{Device, DeviceLimits, TextureFormat};
use engine_render::{bake_brdf_lut, build_brdf_lut_bake_pipeline, contracts};

fn try_device() -> Option<Device> {
    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        Device::new(DeviceLimits::Tier1Minimum, false)
    }));
    match result {
        Ok(Ok(d)) => Some(d),
        Ok(Err(e)) => {
            eprintln!("[init_smoke] no compatible adapter (skipping): {e}");
            None
        }
        Err(_) => {
            eprintln!("[init_smoke] wgpu has no backend enabled (skipping)");
            None
        }
    }
}

/// ADR-075 §1 + ADR-065 §3: bake the BRDF LUT end-to-end against a
/// real device. The returned [`engine_gpu::Texture`] has the expected
/// 512² extent and `Rg16Float` format; wgpu validation at bind-group
/// creation enforces the format match against the shader's
/// `texture_storage_2d<rg16float, write>` declaration.
#[test]
fn bake_brdf_lut_runs_to_completion() {
    let Some(device) = try_device() else {
        return;
    };
    let queue = device.queue();
    let pipeline = build_brdf_lut_bake_pipeline(&device).expect("BRDF LUT bake pipeline builds");
    let lut = bake_brdf_lut(&device, &queue, &pipeline);
    assert_eq!(lut.format(), TextureFormat::Rg16Float);
    let extent = lut.extent();
    assert_eq!(extent.width, contracts::BRDF_LUT_DIM);
    assert_eq!(extent.height, contracts::BRDF_LUT_DIM);
    assert_eq!(extent.depth_or_array_layers, 1);
    // Drop the texture + pipeline cleanly; no further validation
    // beyond the dispatch-and-submit round-trip.
    drop(lut);
    drop(pipeline);
}
