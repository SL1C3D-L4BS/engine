//! SIMD parity oracle (Phase 1, ADR-027).
//!
//! Generates 100k random inputs from a deterministic seed and checks that the
//! type's own arithmetic — which Phase 1 may rewrite to use SIMD intrinsics —
//! produces byte-identical output to the frozen scalar reference in
//! `src/vec_scalar_reference.rs`. Bit-equality is the contract; any
//! divergence localises immediately to the operation under test, which is
//! what makes the SIMD replacement achievable.
//!
//! The oracle is also the regression guardrail: once SIMD lands, an
//! accidental change to reduction order or to a single intrinsic call will
//! fire here long before the cross-architecture determinism oracle notices.

#[path = "../src/vec_scalar_reference.rs"]
mod scalar_reference;

use engine_core::rng::Rng;
use engine_math::{Mat3, Mat4, Quat, Vec2, Vec3, Vec4, vec3, vec4};
use scalar_reference as scalar;

const ITERATIONS: u32 = 100_000;

/// Convert a uniform `[0, 1)` draw to a finite `f32` in roughly
/// `[-1024, 1024]`. Avoids NaN, Inf, and subnormals that would obscure a
/// reduction-order regression behind a single non-finite output.
fn finite_f32(rng: &mut Rng, channel: &str) -> f32 {
    let u = rng.next_f32(channel);
    (u - 0.5) * 2048.0
}

fn rand_vec2(rng: &mut Rng) -> Vec2 {
    Vec2::new(finite_f32(rng, "vec2.x"), finite_f32(rng, "vec2.y"))
}

fn rand_vec3(rng: &mut Rng) -> Vec3 {
    vec3(
        finite_f32(rng, "vec3.x"),
        finite_f32(rng, "vec3.y"),
        finite_f32(rng, "vec3.z"),
    )
}

fn rand_vec4(rng: &mut Rng) -> Vec4 {
    vec4(
        finite_f32(rng, "vec4.x"),
        finite_f32(rng, "vec4.y"),
        finite_f32(rng, "vec4.z"),
        finite_f32(rng, "vec4.w"),
    )
}

fn rand_mat3(rng: &mut Rng) -> Mat3 {
    let mut m = [0.0f32; 9];
    for (i, slot) in m.iter_mut().enumerate() {
        *slot = finite_f32(rng, "mat3") * (1.0 + (i as f32) * 1e-4);
    }
    Mat3::from_cols_array(m)
}

fn rand_mat4(rng: &mut Rng) -> Mat4 {
    let mut m = [0.0f32; 16];
    for (i, slot) in m.iter_mut().enumerate() {
        *slot = finite_f32(rng, "mat4") * (1.0 + (i as f32) * 1e-4);
    }
    Mat4::from_cols_array(m)
}

fn assert_f32_bits_eq(label: &str, lhs: f32, rhs: f32) {
    let lb = lhs.to_bits();
    let rb = rhs.to_bits();
    assert_eq!(
        lb, rb,
        "{label}: simd {lhs:e} (bits {lb:#010x}) != scalar {rhs:e} (bits {rb:#010x})",
    );
}

fn assert_vec3_bits_eq(label: &str, lhs: Vec3, rhs: Vec3) {
    assert_f32_bits_eq(&format!("{label}.x"), lhs.x, rhs.x);
    assert_f32_bits_eq(&format!("{label}.y"), lhs.y, rhs.y);
    assert_f32_bits_eq(&format!("{label}.z"), lhs.z, rhs.z);
}

fn assert_vec4_bits_eq(label: &str, lhs: Vec4, rhs: Vec4) {
    assert_f32_bits_eq(&format!("{label}.x"), lhs.x, rhs.x);
    assert_f32_bits_eq(&format!("{label}.y"), lhs.y, rhs.y);
    assert_f32_bits_eq(&format!("{label}.z"), lhs.z, rhs.z);
    assert_f32_bits_eq(&format!("{label}.w"), lhs.w, rhs.w);
}

fn assert_mat3_bits_eq(label: &str, lhs: Mat3, rhs: Mat3) {
    for (i, (a, b)) in lhs
        .to_cols_array()
        .iter()
        .zip(rhs.to_cols_array().iter())
        .enumerate()
    {
        assert_f32_bits_eq(&format!("{label}[{i}]"), *a, *b);
    }
}

fn assert_mat4_bits_eq(label: &str, lhs: Mat4, rhs: Mat4) {
    for (i, (a, b)) in lhs
        .to_cols_array()
        .iter()
        .zip(rhs.to_cols_array().iter())
        .enumerate()
    {
        assert_f32_bits_eq(&format!("{label}[{i}]"), *a, *b);
    }
}

#[test]
fn vec_arithmetic_matches_scalar_reference() {
    let mut rng = Rng::new(0x5C1D_E0E0_0001, 1);
    for i in 0..ITERATIONS {
        let a2 = rand_vec2(&mut rng);
        let b2 = rand_vec2(&mut rng);
        let a3 = rand_vec3(&mut rng);
        let b3 = rand_vec3(&mut rng);
        let a4 = rand_vec4(&mut rng);
        let b4 = rand_vec4(&mut rng);
        let s = finite_f32(&mut rng, "scalar");

        let exp_v2 = scalar::vec2_add(a2, b2);
        let got_v2 = a2 + b2;
        assert_f32_bits_eq(&format!("vec2_add[{i}].x"), got_v2.x, exp_v2.x);
        assert_f32_bits_eq(&format!("vec2_add[{i}].y"), got_v2.y, exp_v2.y);

        assert_vec3_bits_eq(&format!("vec3_add[{i}]"), a3 + b3, scalar::vec3_add(a3, b3));
        assert_vec4_bits_eq(&format!("vec4_add[{i}]"), a4 + b4, scalar::vec4_add(a4, b4));
        assert_vec3_bits_eq(&format!("vec3_sub[{i}]"), a3 - b3, scalar::vec3_sub(a3, b3));
        assert_vec4_bits_eq(&format!("vec4_sub[{i}]"), a4 - b4, scalar::vec4_sub(a4, b4));
        assert_vec3_bits_eq(
            &format!("vec3_mul_scalar[{i}]"),
            a3 * s,
            scalar::vec3_mul_scalar(a3, s),
        );
        assert_vec4_bits_eq(
            &format!("vec4_mul_scalar[{i}]"),
            a4 * s,
            scalar::vec4_mul_scalar(a4, s),
        );
        assert_vec3_bits_eq(&format!("vec3_mul[{i}]"), a3 * b3, scalar::vec3_mul(a3, b3));
        assert_vec4_bits_eq(&format!("vec4_mul[{i}]"), a4 * b4, scalar::vec4_mul(a4, b4));
        if s != 0.0 {
            assert_vec3_bits_eq(
                &format!("vec3_div_scalar[{i}]"),
                a3 / s,
                scalar::vec3_div_scalar(a3, s),
            );
            assert_vec4_bits_eq(
                &format!("vec4_div_scalar[{i}]"),
                a4 / s,
                scalar::vec4_div_scalar(a4, s),
            );
        }
        assert_vec3_bits_eq(&format!("vec3_neg[{i}]"), -a3, scalar::vec3_neg(a3));
        assert_vec4_bits_eq(&format!("vec4_neg[{i}]"), -a4, scalar::vec4_neg(a4));
    }
}

#[test]
fn vec_dot_cross_match_scalar_reference() {
    let mut rng = Rng::new(0x5C1D_E0E0_0002, 1);
    for i in 0..ITERATIONS {
        let a3 = rand_vec3(&mut rng);
        let b3 = rand_vec3(&mut rng);
        let a4 = rand_vec4(&mut rng);
        let b4 = rand_vec4(&mut rng);
        assert_f32_bits_eq(
            &format!("vec3_dot[{i}]"),
            a3.dot(b3),
            scalar::vec3_dot(a3, b3),
        );
        assert_f32_bits_eq(
            &format!("vec4_dot[{i}]"),
            a4.dot(b4),
            scalar::vec4_dot(a4, b4),
        );
        assert_vec3_bits_eq(
            &format!("vec3_cross[{i}]"),
            a3.cross(b3),
            scalar::vec3_cross(a3, b3),
        );
    }
}

#[test]
fn mat_multiplies_match_scalar_reference() {
    let mut rng = Rng::new(0x5C1D_E0E0_0003, 1);
    // Matrices are heavier — 100k full mat4 multiplies is overkill; 10k is
    // plenty for parity (every cell exercises the same code path).
    let n = ITERATIONS / 10;
    for i in 0..n {
        let a3 = rand_mat3(&mut rng);
        let b3 = rand_mat3(&mut rng);
        let a4 = rand_mat4(&mut rng);
        let b4 = rand_mat4(&mut rng);
        let v = rand_vec4(&mut rng);

        assert_mat3_bits_eq(&format!("mat3_mul[{i}]"), a3 * b3, scalar::mat3_mul(a3, b3));
        assert_mat4_bits_eq(&format!("mat4_mul[{i}]"), a4 * b4, scalar::mat4_mul(a4, b4));
        assert_vec4_bits_eq(
            &format!("mat4_mul_vec[{i}]"),
            a4 * v,
            scalar::mat4_mul_vec(a4, v),
        );
    }
    // The unused `Quat` import is intentional — keep the parity oracle
    // ready to absorb a quaternion path the moment any of it goes SIMD.
    let _ = Quat::IDENTITY;
}
