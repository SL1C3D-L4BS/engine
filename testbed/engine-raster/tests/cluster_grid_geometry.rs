//! ADR-043 verification — deterministic cluster-slice geometry.
//!
//! Asserts that the logarithmic depth distribution exactly matches the
//! closed-form expression `slice_z(i) = z_near · (z_far / z_near)^(i/N)`
//! for the engine's standard 24-slice grid, that consecutive slices
//! cover the full `[z_near, z_far]` interval with no gaps, and that the
//! flat cell-index formula is a bijection over the `16 × 9 × 24` grid.

use engine_raster::{
    CLUSTER_CELL_COUNT, CLUSTER_SLICES, CLUSTER_TILES_X, CLUSTER_TILES_Y, cell_index, slice_z,
};

#[test]
fn slice_z_matches_closed_form_for_standard_frustum() {
    let near = 0.1f32;
    let far = 1000.0f32;
    let ratio = far / near;
    for i in 0..=CLUSTER_SLICES {
        let expected = near * ratio.powf((i as f32) / (CLUSTER_SLICES as f32));
        let got = slice_z(i, near, far);
        let rel_err = ((got - expected) / expected).abs();
        assert!(rel_err < 1e-5, "slice {i}: got {got}, expected {expected}");
    }
}

#[test]
fn slice_boundaries_cover_full_range_without_gaps() {
    let near = 0.5f32;
    let far = 250.0f32;
    let mut prev = slice_z(0, near, far);
    assert!((prev - near).abs() < 1e-4);
    for i in 1..=CLUSTER_SLICES {
        let z = slice_z(i, near, far);
        assert!(z > prev, "slice {i} not monotone: {prev} → {z}");
        prev = z;
    }
    assert!((prev - far).abs() < 0.05);
}

#[test]
fn cell_index_is_a_bijection_over_the_grid() {
    let mut seen = vec![false; CLUSTER_CELL_COUNT as usize];
    let mut count = 0u32;
    for slice in 0..CLUSTER_SLICES {
        for ty in 0..CLUSTER_TILES_Y {
            for tx in 0..CLUSTER_TILES_X {
                let idx = cell_index(tx, ty, slice);
                assert!(!seen[idx as usize], "duplicate cell index {idx}");
                seen[idx as usize] = true;
                count += 1;
            }
        }
    }
    assert_eq!(count, CLUSTER_CELL_COUNT);
    assert!(seen.into_iter().all(|b| b), "cell index is not surjective");
}

#[test]
fn cluster_grid_total_matches_spec() {
    // Spec §IV.4.A line 381: "Cluster assignment compute, 16×9×24 tile-slice grid."
    assert_eq!(CLUSTER_TILES_X, 16);
    assert_eq!(CLUSTER_TILES_Y, 9);
    assert_eq!(CLUSTER_SLICES, 24);
    assert_eq!(CLUSTER_CELL_COUNT, 16 * 9 * 24);
    assert_eq!(CLUSTER_CELL_COUNT, 3_456);
}
