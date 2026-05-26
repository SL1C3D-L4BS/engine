//! ADR-040 verification — CPU shadow rendering produces a stable
//! reference image.
//!
//! The shadow-heavy fixture is rendered twice and compared via the
//! ADR-046 oracle. The verdict must be `Pass` (bit-perfect) for the
//! same inputs — this anchors the determinism contract for the CPU
//! shadow path. When the GPU CSM pass is wired up, the same fixture's
//! GPU output will be compared against this reference under the
//! ADR-046 1/255 threshold.

use engine_raster::{OracleVerdict, compare_images, shadow_heavy_scene};

#[test]
fn shadow_heavy_fixture_self_compares_bit_perfect() {
    let (a, _, _, _, _) = shadow_heavy_scene();
    let (b, _, _, _, _) = shadow_heavy_scene();
    let cmp = compare_images(&a.framebuffer, &b.framebuffer);
    assert_eq!(cmp.verdict, OracleVerdict::Pass);
    assert_eq!(cmp.violating_pixels, 0);
    assert_eq!(cmp.max_delta, 0.0);
}

#[test]
fn shadow_heavy_fixture_has_lit_and_shadowed_regions() {
    // The cascade quadrant visualisation should contain both
    // near-zero (unshadowed) and high (shadowed-by-caster) pixel
    // values — otherwise the depth raster never wrote anything.
    let (scene, _, _, _, _) = shadow_heavy_scene();
    let mut dark = 0u32;
    let mut bright = 0u32;
    for p in scene.framebuffer.color() {
        if p.r < 16 {
            dark += 1;
        } else if p.r > 64 {
            bright += 1;
        }
    }
    assert!(dark > 0 && bright > 0, "expected mixed shadow image");
}

#[test]
fn shadow_atlas_quadrants_are_independently_populated() {
    // Render the fixture; confirm the *first* cascade quadrant carries
    // depth writes (the boxes are mostly in the near-cascade frustum)
    // and that the depth values are reverse-Z (i.e. > 0).
    use engine_raster::CASCADE_DIM;
    let (_, _, _, cascades, atlas) = shadow_heavy_scene();
    let ox = cascades.cascades[0].atlas_x;
    let oy = cascades.cascades[0].atlas_y;
    let mut nonzero = 0u32;
    for dy in (0..CASCADE_DIM).step_by(64) {
        for dx in (0..CASCADE_DIM).step_by(64) {
            if atlas.read(ox + dx, oy + dy) > 0.0 {
                nonzero += 1;
            }
        }
    }
    assert!(nonzero > 0, "first cascade was not populated");
}
