//! ADR-043 verification — CPU reference cluster assignment over a
//! synthetic 100-light scene.
//!
//! Per ADR-043 §Verification: "synthetic 100-light scene on a known
//! camera; CPU reference cluster assignment vs GPU output. Per-cell
//! light-id sets must match (order may differ)."
//!
//! Without a GPU runner available in unit-test CI, this test exercises
//! the CPU reference path twice with the same inputs and asserts the
//! per-cell light-id *sets* are identical (the same determinism the
//! GPU comparison will be measured against). The CPU implementation
//! also satisfies the "indices appended in deterministic order"
//! property, so a stronger sequence-equality check passes for the
//! reference-vs-reference run.

use engine_math::Vec3;
use engine_raster::{Camera, Light, MAX_LIGHTS_PER_CLUSTER, assign_lights};
use std::collections::HashSet;

fn test_camera() -> Camera {
    Camera {
        position: Vec3::new(0.0, 0.0, 0.0),
        forward: Vec3::new(0.0, 0.0, -1.0),
        up: Vec3::new(0.0, 1.0, 0.0),
        fov_y: 60.0_f32.to_radians(),
        aspect: 16.0 / 9.0,
        near: 0.1,
        far: 200.0,
    }
}

fn synthetic_lights(n: usize) -> Vec<Light> {
    // Deterministic layout: lights on a 10×10 lattice along XZ, all
    // in front of the camera, mild intensity, 4 m range. The lattice
    // covers the visible scene so cluster cells will have measurable
    // light counts.
    let mut lights = Vec::with_capacity(n);
    for i in 0..n {
        let ix = i % 10;
        let iz = i / 10;
        let x = (ix as f32 - 4.5) * 2.0;
        let z = -3.0 - (iz as f32) * 2.0;
        lights.push(Light::point(
            Vec3::new(x, 1.0, z),
            Vec3::new(1.0, 0.9, 0.8),
            1.0,
            4.0,
        ));
    }
    lights
}

#[test]
fn cpu_reference_is_deterministic() {
    let cam = test_camera();
    let lights = synthetic_lights(100);
    let a = assign_lights(&cam, &lights);
    let b = assign_lights(&cam, &lights);
    for (ca, cb) in a.cells().iter().zip(b.cells().iter()) {
        assert_eq!(ca.light_count, cb.light_count);
        for k in 0..ca.light_count as usize {
            assert_eq!(ca.light_indices[k], cb.light_indices[k]);
        }
    }
    assert_eq!(a.overflow_count, b.overflow_count);
}

#[test]
fn per_cell_light_sets_match_under_permuted_input() {
    // ADR-043 §Verification: "Per-cell light-id sets must match
    // (order may differ)". The clause assumes no overflow — when a
    // cell hits the 32-light cap, ADR-043 §4 makes the drop set
    // insertion-order dependent and the equality is intentionally
    // not enforced. We use a light count well below the cap so the
    // permutation invariant holds across the whole grid.
    let cam = test_camera();
    let lights = synthetic_lights(16);
    let mut permuted: Vec<Light> = lights.clone();
    permuted.reverse();
    let original = assign_lights(&cam, &lights);
    let perm = assign_lights(&cam, &permuted);

    for (cell_a, cell_b) in original.cells().iter().zip(perm.cells().iter()) {
        // Translate `cell_b`'s indices back through the reverse
        // permutation so they reference the original light array.
        let set_a: HashSet<u16> = cell_a.light_indices[..cell_a.light_count as usize]
            .iter()
            .copied()
            .collect();
        let set_b: HashSet<u16> = cell_b.light_indices[..cell_b.light_count as usize]
            .iter()
            .map(|&i| (lights.len() as u16 - 1) - i)
            .collect();
        assert_eq!(set_a, set_b, "cell light sets diverged under permutation");
    }
    // Verify the bound: no cell may exceed the cap under this load.
    for cell in original.cells() {
        assert!(cell.light_count as usize <= MAX_LIGHTS_PER_CLUSTER);
    }
    assert_eq!(original.overflow_count, 0);
}

#[test]
fn overflow_counter_records_pathological_cluster_load() {
    let cam = test_camera();
    // Pack 64 point lights into the same small volume — every visible
    // cluster cell exceeds the 32-light cap, driving the overflow
    // counter > 0 (ADR-043 §4).
    let mut lights = Vec::with_capacity(64);
    for _ in 0..64 {
        lights.push(Light::point(
            Vec3::new(0.0, 1.0, -2.0),
            Vec3::new(1.0, 1.0, 1.0),
            1.0,
            8.0,
        ));
    }
    let grid = assign_lights(&cam, &lights);
    assert!(
        grid.overflow_count > 0,
        "overflow counter did not fire under 64 colocated lights"
    );
    // No cell may exceed the cap.
    for cell in grid.cells() {
        assert!(cell.light_count as usize <= MAX_LIGHTS_PER_CLUSTER);
    }
}
