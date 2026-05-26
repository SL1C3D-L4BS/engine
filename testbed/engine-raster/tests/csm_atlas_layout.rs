//! ADR-040 verification — deterministic CSM cascade layout.
//!
//! Asserts that the four cascade view-projection matrices are
//! byte-identical across two builds for the same camera + light + jitter
//! seed (the "same seed → same matrices, byte-equal" clause in
//! ADR-040 §Verification). Also confirms the fixed four-quadrant atlas
//! layout: every cascade lives in a distinct 2048×2048 quadrant of the
//! 4096² atlas.

use engine_math::Vec3;
use engine_raster::{
    ATLAS_DIM, CASCADE_DIM, CSM_CASCADES, Camera, atlas_origin, build_cascades, cascade_splits,
};

fn test_camera() -> Camera {
    Camera {
        position: Vec3::new(0.0, 0.0, 0.0),
        forward: Vec3::new(0.0, 0.0, -1.0),
        up: Vec3::new(0.0, 1.0, 0.0),
        fov_y: 60.0_f32.to_radians(),
        aspect: 16.0 / 9.0,
        near: 0.1,
        far: 1000.0,
    }
}

#[test]
fn cascade_view_projections_are_byte_equal_across_builds() {
    let cam = test_camera();
    let light = Vec3::new(-0.4, -1.0, -0.2);
    let a = build_cascades(&cam, light, (0.0, 0.0));
    let b = build_cascades(&cam, light, (0.0, 0.0));
    for i in 0..CSM_CASCADES {
        let am = a.cascades[i].view_projection.to_cols_array();
        let bm = b.cascades[i].view_projection.to_cols_array();
        for k in 0..16 {
            assert_eq!(
                am[k].to_bits(),
                bm[k].to_bits(),
                "cascade {i} element {k} not bit-equal: {} vs {}",
                am[k],
                bm[k]
            );
        }
    }
}

#[test]
fn cascade_splits_are_monotone_and_cover_full_range() {
    let cam = test_camera();
    let splits = cascade_splits(cam.near, cam.far, 0.6);
    assert!((splits[0] - cam.near).abs() < 1e-6);
    assert!((splits[CSM_CASCADES] - cam.far).abs() < 1e-3);
    for i in 1..=CSM_CASCADES {
        assert!(
            splits[i] > splits[i - 1],
            "splits regressed at {i}: {:?}",
            splits
        );
    }
}

#[test]
fn atlas_quadrants_are_distinct_and_fit() {
    let mut occupied = vec![false; (ATLAS_DIM * ATLAS_DIM) as usize];
    for i in 0..CSM_CASCADES {
        let (ox, oy) = atlas_origin(i);
        // Quadrant must fit inside the atlas.
        assert!(ox + CASCADE_DIM <= ATLAS_DIM);
        assert!(oy + CASCADE_DIM <= ATLAS_DIM);
        for dy in 0..CASCADE_DIM {
            for dx in 0..CASCADE_DIM {
                let idx = ((oy + dy) * ATLAS_DIM + (ox + dx)) as usize;
                assert!(
                    !occupied[idx],
                    "cascade {i} overlaps a previous quadrant at ({}, {})",
                    ox + dx,
                    oy + dy
                );
                occupied[idx] = true;
            }
        }
    }
    // All four quadrants together cover exactly half the atlas (4 × 2048² = 16 M, 4096² = 16 M).
    let covered = occupied.iter().filter(|&&b| b).count() as u64;
    assert_eq!(covered, (ATLAS_DIM as u64) * (ATLAS_DIM as u64));
}

#[test]
fn jitter_offset_shifts_view_projection() {
    let cam = test_camera();
    let light = Vec3::new(-0.4, -1.0, -0.2);
    let a = build_cascades(&cam, light, (0.0, 0.0));
    let b = build_cascades(&cam, light, (0.5, 0.5));
    // At least one matrix element must differ when jitter is applied
    // (otherwise the cascade ↔ TAA contract is broken).
    let mut any_diff = false;
    for i in 0..CSM_CASCADES {
        let am = a.cascades[i].view_projection.to_cols_array();
        let bm = b.cascades[i].view_projection.to_cols_array();
        for k in 0..16 {
            if am[k].to_bits() != bm[k].to_bits() {
                any_diff = true;
                break;
            }
        }
    }
    assert!(any_diff, "jitter ignored — cascade VPs are identical");
}
