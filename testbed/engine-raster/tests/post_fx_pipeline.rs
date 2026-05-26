//! End-to-end post-FX chain oracle (ADR-042 §Verification).
//!
//! Runs the canonical post-FX order against a synthetic HDR scene:
//! `SSAO → TAA → Bloom → Tonemap`. The test exercises pipeline wiring
//! more than the per-stage math (each stage's primitives have unit
//! tests in `engine_raster::post_fx::tests`).

use engine_math::{Vec2, Vec3};
use engine_raster::post_fx::{
    TaaInput, bloom_composite, bloom_extract, gaussian_blur_3x3, ssao_apply, taa_resolve,
    tonemap_aces,
};

const W: u32 = 8;
const H: u32 = 8;

fn fill(r: f32, g: f32, b: f32) -> Vec<Vec3> {
    vec![Vec3::new(r, g, b); (W * H) as usize]
}

#[test]
fn end_to_end_chain_produces_ldr_output_with_bloom_contribution() {
    // Synthetic HDR scene: a single bright pixel at the centre, dim
    // background. After bloom + tonemap the centre pixel saturates,
    // its neighbours pick up bloom energy, the background remains dim
    // but visible.
    let mut hdr = fill(0.05, 0.05, 0.05);
    let centre = (H / 2) * W + (W / 2);
    hdr[centre as usize] = Vec3::new(8.0, 8.0, 8.0);

    // SSAO over flat depth — should leave the image untouched.
    let depth = vec![10.0_f32; (W * H) as usize];
    ssao_apply(&mut hdr, &depth, W, H, 2, 1.0);
    assert!((hdr[0].x - 0.05).abs() < 1e-4, "flat-depth SSAO must be ~1");

    // TAA: history = current → near-identity.
    let history = hdr.clone();
    let motion = vec![Vec2::ZERO; (W * H) as usize];
    let depth_hist = depth.clone();
    let mut resolved = vec![Vec3::ZERO; (W * H) as usize];
    let input = TaaInput {
        current: &hdr,
        history: &history,
        motion: &motion,
        depth_current: &depth,
        depth_history: &depth_hist,
        width: W,
        height: H,
    };
    taa_resolve(&input, &mut resolved);
    assert!((resolved[0].x - hdr[0].x).abs() < 1e-4);
    assert!((resolved[centre as usize].x - hdr[centre as usize].x).abs() < 0.5);

    // Bloom: extract bright pixels + blur. Threshold 1.0 zeroes the
    // dim background; the centre survives.
    let bright: Vec<Vec3> = resolved.iter().map(|c| bloom_extract(*c, 1.0)).collect();
    assert_eq!(bright[0], Vec3::ZERO);
    assert!(bright[centre as usize].x > 0.0);

    let mut blurred = vec![Vec3::ZERO; (W * H) as usize];
    gaussian_blur_3x3(&bright, &mut blurred, W, H);
    // Energy must spread to the centre's neighbours.
    let neigh = (H / 2) * W + (W / 2) - 1;
    assert!(blurred[neigh as usize].x > 0.0);

    // Composite + tonemap.
    let mut tonemapped = vec![Vec3::ZERO; (W * H) as usize];
    for i in 0..(W * H) as usize {
        let composite = bloom_composite(resolved[i], blurred[i], 0.5);
        tonemapped[i] = tonemap_aces(composite);
    }
    // Centre tone-maps near unity.
    assert!(tonemapped[centre as usize].x > 0.9);
    // Background dim but non-zero.
    assert!(tonemapped[0].x > 0.0);
    assert!(tonemapped[0].x < 0.2);
}
