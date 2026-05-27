//! Cross-checks: engine-render's GPU-side contract constants
//! (ADR-064 + ADR-065) must match this testbed's CPU oracle
//! constants. A drift between the two would silently desync the
//! pixel-parity oracle when the GPU `record()` bodies land in
//! Phase 6 PR 3.5 / 4.5.

use engine_raster::cluster::{
    CLUSTER_CELL_COUNT, CLUSTER_SLICES, CLUSTER_TILES_X, CLUSTER_TILES_Y, MAX_LIGHTS_PER_CLUSTER,
};
use engine_raster::ibl::MAX_PROBES;
use engine_raster::shadow::{ATLAS_DIM, CASCADE_DIM, CSM_CASCADES, PRACTICAL_SPLIT_LAMBDA};
use engine_render::contracts;

#[test]
fn cluster_grid_constants_match_oracle() {
    assert_eq!(contracts::CLUSTER_TILES_X, CLUSTER_TILES_X);
    assert_eq!(contracts::CLUSTER_TILES_Y, CLUSTER_TILES_Y);
    assert_eq!(contracts::CLUSTER_TILES_Z, CLUSTER_SLICES);
    assert_eq!(contracts::CLUSTER_CELL_COUNT, CLUSTER_CELL_COUNT);
}

#[test]
fn cluster_light_cap_matches_oracle() {
    assert_eq!(
        contracts::MAX_LIGHTS_PER_CLUSTER as usize,
        MAX_LIGHTS_PER_CLUSTER
    );
}

#[test]
fn csm_constants_match_oracle() {
    assert_eq!(contracts::CSM_CASCADES_CONTRACT, CSM_CASCADES);
    assert_eq!(contracts::CSM_ATLAS_DIM, ATLAS_DIM);
    assert_eq!(contracts::CSM_CASCADE_DIM, CASCADE_DIM);
    assert!(
        (contracts::CSM_PRACTICAL_SPLIT_LAMBDA - PRACTICAL_SPLIT_LAMBDA).abs() < f32::EPSILON,
        "λ mismatch: contract={} oracle={}",
        contracts::CSM_PRACTICAL_SPLIT_LAMBDA,
        PRACTICAL_SPLIT_LAMBDA
    );
}

#[test]
fn ibl_probe_cap_matches_oracle() {
    assert_eq!(contracts::MAX_IBL_PROBES as usize, MAX_PROBES);
}

#[test]
fn cluster_workgroup_size_aligns_with_grid_xy() {
    // The cluster-assignment compute pass (ADR-064 §5) dispatches one
    // thread per (x, y) cluster column; the z slices are walked inside
    // the kernel. The workgroup size must match the grid X×Y extents
    // so a single dispatch covers every column.
    assert_eq!(contracts::CLUSTER_ASSIGN_WORKGROUP_SIZE[0], CLUSTER_TILES_X);
    assert_eq!(contracts::CLUSTER_ASSIGN_WORKGROUP_SIZE[1], CLUSTER_TILES_Y);
    assert_eq!(contracts::CLUSTER_ASSIGN_WORKGROUP_SIZE[2], 1);
}

#[test]
fn push_constants_size_is_portable() {
    // 64 B is the cap that ships on every backend the engine targets
    // (ADR-063 §5). The const-assert in contracts.rs already enforces
    // this at compile time; the runtime assertion gives a friendlier
    // failure message if a refactor breaks it.
    assert_eq!(core::mem::size_of::<contracts::PushConstants>(), 64);
}

#[test]
fn light_record_size_is_64_bytes() {
    // 4 × vec4<f32> = 64 B, the standard PBR light record layout
    // (ADR-064 §5). Matches the CPU oracle's effective per-light
    // memory cost.
    assert_eq!(core::mem::size_of::<contracts::LightRecord>(), 64);
}
