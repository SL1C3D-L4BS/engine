//! Phase 6 PR 7 — GPU pipeline smoke test.
//!
//! Construct a fallback-adapter [`engine_gpu::Device`] and run every
//! embedded WGSL shader through [`build_all_phase6_pipelines`]. If
//! wgpu rejects any pipeline assembly (shader-validation error,
//! bind-group-layout mismatch, missing feature), the test fails
//! loudly and the failure is the PR-8 cue to populate that pass's
//! real bind-group / vertex-layout / push-constant descriptors.
//!
//! Marked `#[ignore]` so the default `cargo test --workspace` stays
//! green on environments that lack a GPU adapter (CI workers without
//! Vulkan / Metal / DX12 backends). The workspace pins
//! `wgpu = { default-features = false, features = ["wgsl"] }`, so
//! actually exercising pipeline construction needs a backend feature
//! propagated through — see the PR-7 addendum on ADR-068 for the
//! `[patch.crates-io]` block + `wgpu` feature toggles used on the
//! self-hosted RX 6700 XT runner (ADR-047 §2). Without that, the
//! test skips with the "no backend enabled" log line below rather
//! than failing.
//!
//! ```text
//! cargo test -p engine-render --test pipeline_smoke -- --ignored
//! ```

use std::panic::{self, AssertUnwindSafe};

use engine_gpu::{Device, DeviceLimits};
use engine_render::build_all_phase6_pipelines;

/// Try to construct a fallback-adapter device. Returns `None` when no
/// adapter is available (the test should skip rather than fail).
///
/// wgpu 29 panics — not errors — when no backend feature is enabled
/// at build time. We treat that as a skip signal too: the test runs
/// only when the workspace is built with at least one wgpu backend
/// active (e.g. `wgpu/vulkan` on Linux, `wgpu/metal` on macOS).
fn try_fallback_device() -> Option<Device> {
    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        Device::new(DeviceLimits::Tier1Minimum, true)
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

#[test]
#[ignore = "requires a wgpu-compatible adapter; run with --ignored"]
fn build_all_phase6_pipelines_against_fallback_device() {
    let Some(device) = try_fallback_device() else {
        return;
    };
    let bundle = build_all_phase6_pipelines(&device);

    // Smoke-only: confirm the bundle returned without panicking.
    // (Drop happens at end-of-scope; pipelines tear down cleanly.)
    drop(bundle);
}
