//! Phase 5.5 A.3 — pixel-parity oracle fixtures (ADR-046).
//!
//! Each fixture renders a known scene through (a) the CPU oracle in
//! `engine-raster` and (b) the GPU render graph on the user's RX 580,
//! then diffs the framebuffers per ADR-046 thresholds (1/255 per
//! channel in linear space, p99 ≤ 1% pixels exceeding threshold,
//! max delta ≤ 4/255).
//!
//! Cargo treats this top-level `tests/pixel_parity.rs` as the
//! integration test binary; the `#[path]` attributes below attach
//! the shared harness and each fixture file from the sibling
//! `tests/pixel_parity/` directory. Without `#[path]`, rustc would
//! look for `tests/harness.rs` (no, we don't want that — multiple
//! integration tests would be auto-discovered by Cargo and run twice).

#[path = "pixel_parity/harness.rs"]
mod harness;

#[path = "pixel_parity/common.rs"]
mod common;

#[path = "pixel_parity/cube.rs"]
mod cube;

#[path = "pixel_parity/csm_4_cascade.rs"]
mod csm_4_cascade;

#[path = "pixel_parity/cluster_64_lights.rs"]
mod cluster_64_lights;

#[path = "pixel_parity/ibl_probe.rs"]
mod ibl_probe;

#[path = "pixel_parity/taa_motion.rs"]
mod taa_motion;

#[path = "pixel_parity/post_fx_chain.rs"]
mod post_fx_chain;
