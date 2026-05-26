//! IBL L2 SH convolution oracle (ADR-041 §Verification).
//!
//! The Ramamoorthi-Hanrahan closed form for the L2 SH reconstruction of
//! cosine-convolved irradiance from a unit-white directional light is:
//!
//! ```text
//! E(n) / L = 1/4 + (n·d)/2 + (5/32) · (3 (n·d)² − 1)
//! ```
//!
//! This test sweeps `(d, n)` pairs and asserts the [`ShL2`] numeric
//! evaluation matches the analytic reference to within 1e-4 — the
//! tightest determinism threshold the L2 truncation permits.

use engine_math::Vec3;
use engine_raster::ibl::{ShL2, directional_light_irradiance_closed_form};

fn vec3(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3::new(x, y, z)
}

#[test]
fn sweep_cosine_angle_matches_closed_form() {
    // 16 light directions × 16 surface normals, all on the lit
    // hemisphere where the unclamped analytic matches the numeric
    // evaluation byte-for-byte.
    let n_dirs = 16;
    for li in 0..n_dirs {
        let theta_l = (li as f32) * (core::f32::consts::TAU / n_dirs as f32);
        let d = vec3(theta_l.cos() * 0.4, 0.85, theta_l.sin() * 0.4).normalize_or_zero();
        let sh = ShL2::from_directional_light(d, Vec3::ONE);
        for ni in 0..n_dirs {
            let theta_n = (ni as f32) * (core::f32::consts::TAU / n_dirs as f32);
            let n = vec3(theta_n.cos() * 0.45, 0.78, theta_n.sin() * 0.45).normalize_or_zero();
            let analytic = directional_light_irradiance_closed_form(d, n);
            // Stay in the regime where the L2 reconstruction is
            // non-negative (analytic > 0).
            if analytic <= 0.0 {
                continue;
            }
            let numeric = sh.evaluate_irradiance(n);
            assert!(
                (numeric.x - analytic).abs() < 1e-4,
                "li={li} ni={ni}: numeric {numeric:?} vs analytic {analytic}"
            );
            assert!(
                (numeric.y - analytic).abs() < 1e-4,
                "li={li} ni={ni} y: numeric {numeric:?} vs analytic {analytic}"
            );
            assert!(
                (numeric.z - analytic).abs() < 1e-4,
                "li={li} ni={ni} z: numeric {numeric:?} vs analytic {analytic}"
            );
        }
    }
}

#[test]
fn coloured_light_scales_linearly() {
    // The SH expansion is linear in the light's radiance — verifying
    // this property closes off scalar-decomposition regressions.
    let d = vec3(0.2, 0.95, 0.1).normalize_or_zero();
    let n = vec3(0.1, 0.85, -0.2).normalize_or_zero();
    let sh_white = ShL2::from_directional_light(d, Vec3::ONE);
    let sh_red = ShL2::from_directional_light(d, vec3(0.7, 0.0, 0.0));
    let white_e = sh_white.evaluate_irradiance(n);
    let red_e = sh_red.evaluate_irradiance(n);
    assert!((red_e.x - 0.7 * white_e.x).abs() < 1e-5);
    assert!(red_e.y.abs() < 1e-5);
    assert!(red_e.z.abs() < 1e-5);
}
