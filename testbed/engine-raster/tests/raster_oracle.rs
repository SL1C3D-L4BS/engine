//! Rasterizer-oracle end-to-end test (ADR-046).
//!
//! Renders the golden scene twice and asserts a bit-perfect
//! comparison. This is the verification contract the GPU pipeline
//! (Phase 5 PR 2+) will be measured against: the same scene fed to
//! the GPU passes must round-trip through the oracle to a verdict
//! of `Pass` or `PassUnderThreshold` (with documented exception
//! register entries for the latter).

use engine_raster::{OracleVerdict, compare_images, golden_triangle_scene};

#[test]
fn golden_scene_self_compares_bit_perfect() {
    let a = golden_triangle_scene();
    let b = golden_triangle_scene();
    let cmp = compare_images(&a.framebuffer, &b.framebuffer);
    assert_eq!(cmp.verdict, OracleVerdict::Pass);
    assert_eq!(cmp.violating_pixels, 0);
    assert_eq!(cmp.max_delta, 0.0);
}

#[test]
fn golden_scene_has_expected_pixel_coverage() {
    // Snapshot-style assertion: confirm the rasterizer produced a
    // recognisable triangle. The triangle covers roughly 50% of the
    // viewport area; we accept 35–65% to absorb sub-pixel coverage
    // differences.
    let scene = golden_triangle_scene();
    let fb = &scene.framebuffer;
    let total = (fb.width() * fb.height()) as u64;
    let lit = fb
        .color()
        .iter()
        .filter(|p| p.r > 0 || p.g > 0 || p.b > 0)
        .count() as u64;
    let frac = (lit as f64) / (total as f64);
    assert!(
        frac > 0.35 && frac < 0.65,
        "expected ~50% coverage, got {:.2}% ({lit}/{total})",
        frac * 100.0
    );
}

#[test]
fn rendering_two_overlapping_triangles_obeys_depth() {
    use engine_raster::framebuffer::Rgba8;
    use engine_raster::{Framebuffer, Vertex, Viewport, rasterize_triangle};

    let mut fb = Framebuffer::new(64, 64);
    fb.clear(Rgba8::default(), 1.0);
    let vp = Viewport::fullframe(&fb);

    // Far green triangle.
    let far = [
        Vertex::new(-0.9, -0.9, 0.5, 1.0, 0.0, 1.0, 0.0),
        Vertex::new(0.9, -0.9, 0.5, 1.0, 0.0, 1.0, 0.0),
        Vertex::new(0.0, 0.9, 0.5, 1.0, 0.0, 1.0, 0.0),
    ];
    rasterize_triangle(&mut fb, vp, far);

    // Near red triangle covering a sub-region.
    let near = [
        Vertex::new(-0.5, -0.5, -0.5, 1.0, 1.0, 0.0, 0.0),
        Vertex::new(0.5, -0.5, -0.5, 1.0, 1.0, 0.0, 0.0),
        Vertex::new(0.0, 0.5, -0.5, 1.0, 1.0, 0.0, 0.0),
    ];
    rasterize_triangle(&mut fb, vp, near);

    // Pixel near centre should be red (near triangle wins).
    let centre = fb.sample(32, 32);
    assert!(centre.r > 200, "expected red centre, got {centre:?}");
}
