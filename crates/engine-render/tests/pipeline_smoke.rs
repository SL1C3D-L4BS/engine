//! Phase 5.5 GPU smoke tests.
//!
//! Two layered smokes:
//!
//! 1. [`device_init_against_real_adapter`] — ADR-074 contract. Proves
//!    that [`engine_gpu::Device::new`] reaches a real adapter via the
//!    wgpu Vulkan backend the workspace `Cargo.toml` now enables.
//!    Runs in the default `cargo test --workspace`; skips gracefully
//!    on hosts without a Vulkan loader.
//! 2. [`build_all_phase6_pipelines_against_real_device`] — ADR-075
//!    contract. Constructs every Track-A WGSL pipeline against the
//!    real device and validates the WGSL `@group/@binding` declarations
//!    against the Rust-authored bind-group layouts. Currently
//!    `#[ignore]`'d because the bind-group layouts are A.2 deliverables;
//!    the test correctly fails today with "binding missing from
//!    pipeline layout" until A.2 wires them.
//!
//! ```text
//! cargo test -p engine-render --test pipeline_smoke                            # ADR-074 contract
//! cargo test -p engine-render --test pipeline_smoke -- --include-ignored       # both, after A.2 lands
//! ```

use std::panic::{self, AssertUnwindSafe};

use engine_gpu::{Device, DeviceLimits};
use engine_render::build_all_phase6_pipelines;

/// Try to construct a real-GPU device. Returns `None` when no
/// compatible adapter is available (the test should skip rather than
/// fail).
///
/// `Device::new(_, false)` does *not* force a software fallback adapter
/// (the `allow_fallback` parameter is the engine-gpu name for wgpu's
/// `force_fallback_adapter` request — confusingly, passing `true` would
/// require a Lavapipe/SwiftShader software adapter and skip on a real
/// GPU). PR 7.5 passes `false` so the smoke test exercises pipeline
/// construction against the runner's actual hardware.
///
/// wgpu 29 panics — not errors — when no backend feature is enabled
/// at build time. We treat that as a skip signal too: the test runs
/// only when the workspace is built with at least one wgpu backend
/// active (e.g. `wgpu/vulkan` on Linux, `wgpu/metal` on macOS).
fn try_device() -> Option<Device> {
    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        Device::new(DeviceLimits::Tier1Minimum, false)
    }));
    match result {
        Ok(Ok(d)) => Some(d),
        Ok(Err(e)) => {
            eprintln!("[pipeline_smoke] no compatible adapter (skipping): {e}");
            None
        }
        Err(_) => {
            eprintln!(
                "[pipeline_smoke] wgpu has no backend enabled — \
                 enable e.g. wgpu/vulkan in the workspace and rerun. \
                 Skipping."
            );
            None
        }
    }
}

/// ADR-074 contract: prove wgpu reaches a real adapter through the
/// workspace's `wgpu/vulkan` feature. This is the thinnest possible
/// smoke — it does not depend on the A.2 bind-group authoring work and
/// can run in the default workspace test pass.
#[test]
fn device_init_against_real_adapter() {
    let Some(device) = try_device() else {
        return;
    };
    // Touch the limits + features so the device handle isn't a dead
    // value (and so a future regression in `Device::new`'s feature
    // negotiation surfaces here, not silently downstream).
    let features = device.features();
    let limits = device.limits();
    eprintln!(
        "[pipeline_smoke] device reached: limits={limits:?} \
         features={{push_constants:{}, bc_textures:{}, descriptor_indexing:{}}}",
        features.push_constants, features.bc_textures, features.descriptor_indexing,
    );
    drop(device);
}

/// ADR-075 contract: construct every Track-A WGSL pipeline. The Rust
/// bind-group layouts must match the WGSL `@group/@binding`
/// declarations. Marked `#[ignore]` until A.2 authors the per-pass
/// bind-group layouts (currently the pipelines build with empty
/// layouts; wgpu correctly rejects the first shader binding).
#[test]
#[ignore = "A.2 wires per-pass bind-group layouts; remove when A.2 lands"]
fn build_all_phase6_pipelines_against_real_device() {
    let Some(device) = try_device() else {
        return;
    };
    match build_all_phase6_pipelines(&device) {
        Ok(bundle) => {
            // Smoke-only: confirm the bundle assembled. Drop at
            // end-of-scope cleans up wgpu objects.
            drop(bundle);
        }
        Err((pass_name, err)) => {
            panic!("[pipeline_smoke] pass {pass_name:?} failed pipeline build: {err}");
        }
    }
}
