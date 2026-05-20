//! Cross-architecture determinism oracle for `engine-math`.
//!
//! A fixed battery of operations is reduced to a 64-bit FNV-1a digest of the
//! raw result bits. The digest is asserted against a committed golden file.
//! Because every architecture compares against the *same* golden, two passing
//! runs (x86-64 and aarch64 in CI) are byte-identical to each other — this is
//! the Determinism Contract proven (spec IV.2, ADR-013).
//!
//! Regenerate the golden after an intentional change:
//! `ENGINE_GOLDEN_WRITE=1 cargo test -p engine-math --test determinism`
//! (or `just gen-golden`).

use engine_math::transcendental::*;
use engine_math::*;

/// Incremental FNV-1a 64-bit digest over raw value bits.
struct Digest {
    hash: u64,
}

impl Digest {
    fn new() -> Self {
        Self {
            hash: 0xcbf2_9ce4_8422_2325,
        }
    }

    fn byte(&mut self, b: u8) {
        self.hash ^= b as u64;
        self.hash = self.hash.wrapping_mul(0x0000_0100_0000_01b3);
    }

    fn bytes(&mut self, bs: &[u8]) {
        for &b in bs {
            self.byte(b);
        }
    }

    /// NaN payload bits are not fully specified by IEEE-754, so every NaN is
    /// canonicalized before hashing. Infinities and signed zeros *are*
    /// specified and are hashed as-is.
    fn push_f32(&mut self, v: f32) {
        let bits = if v.is_nan() { 0x7fc0_0000 } else { v.to_bits() };
        self.bytes(&bits.to_le_bytes());
    }

    fn push_f64(&mut self, v: f64) {
        let bits = if v.is_nan() {
            0x7ff8_0000_0000_0000
        } else {
            v.to_bits()
        };
        self.bytes(&bits.to_le_bytes());
    }

    fn push_i64(&mut self, v: i64) {
        self.bytes(&v.to_le_bytes());
    }
}

fn compute() -> u64 {
    let mut d = Digest::new();

    // --- transcendentals: dense sweep -----------------------------------
    let mut i = -2000i32;
    while i < 2000 {
        let x = i as f64 * 0.01;
        d.push_f64(sin_f64(x));
        d.push_f64(cos_f64(x));
        d.push_f64(tan_f64(x));
        d.push_f64(atan_f64(x));
        d.push_f64(atan2_f64(x, 1.0 - x));
        d.push_f64(acos_f64(x * 0.0005));
        d.push_f64(asin_f64(x * 0.0005));
        if x > 0.0 {
            d.push_f64(ln_f64(x));
        }
        if x.abs() < 60.0 {
            d.push_f64(exp_f64(x));
        }
        let xf = i as f32 * 0.01;
        d.push_f32(sin_f32(xf));
        d.push_f32(cos_f32(xf));
        d.push_f32(atan_f32(xf));
        i += 1;
    }

    // --- vectors --------------------------------------------------------
    let mut acc = Vec3::ZERO;
    let mut k = 0i32;
    while k < 600 {
        let f = k as f32;
        let a = vec3(f * 0.013, sin_f32(f * 0.1), -f * 0.007);
        let b = vec3(cos_f32(f * 0.05), f * 0.002, sin_f32(f * 0.03));
        acc = acc + a.cross(b) + a * b.dot(a);
        acc = acc.normalize_or_zero() * (acc.length() + 1.0);
        k += 1;
    }
    d.push_f32(acc.x);
    d.push_f32(acc.y);
    d.push_f32(acc.z);

    // --- matrices: bounded rotation/translation chain -------------------
    let mut m = Mat4::IDENTITY;
    let axis = vec3(0.0, 1.0, 0.0);
    let mut k = 0i32;
    while k < 256 {
        let f = k as f32 * 0.01;
        m = m * Mat4::from_trs(
            vec3(sin_f32(f), cos_f32(f), f * 0.001),
            Quat::from_axis_angle(axis, f * 0.02),
            Vec3::ONE,
        );
        k += 1;
    }
    for v in m.to_cols_array() {
        d.push_f32(v);
    }
    let well_conditioned = Mat4::from_trs(
        vec3(1.0, 2.0, 3.0),
        Quat::from_axis_angle(Vec3::Z, 0.5),
        vec3(2.0, 3.0, 4.0),
    );
    for v in well_conditioned
        .inverse()
        .expect("invertible")
        .to_cols_array()
    {
        d.push_f32(v);
    }

    // --- quaternion slerp ----------------------------------------------
    let q0 = Quat::IDENTITY;
    let q1 = Quat::from_axis_angle(vec3(1.0, 1.0, 1.0).normalize_or_zero(), 2.0);
    let mut k = 0i32;
    while k <= 100 {
        let t = k as f32 * 0.01;
        let rotated = q0.slerp(q1, t).rotate(vec3(1.0, 0.0, 0.0));
        d.push_f32(rotated.x);
        d.push_f32(rotated.y);
        d.push_f32(rotated.z);
        k += 1;
    }

    // --- fixed-point ----------------------------------------------------
    let mut a = I32F32::from_int(1);
    let step = I32F32::from_f64(1.0001);
    let mut k = 0i32;
    while k < 1000 {
        a = a * step + I32F32::from_raw(7);
        d.push_i64(a.to_raw());
        k += 1;
    }
    let mut b = I16F16::from_int(100);
    let div = I16F16::from_f64(1.5);
    let mut k = 0i32;
    while k < 500 {
        b = b / div + I16F16::from_raw(3);
        d.push_i64(b.to_raw() as i64);
        k += 1;
    }

    // --- deterministic scalar wrapper -----------------------------------
    let mut s = F32Det::new(0.5);
    let mut k = 0i32;
    while k < 200 {
        s = (s + F32Det::ONE) * F32Det::new(0.5);
        s = s.sin().abs() + F32Det::new(0.1);
        d.push_f32(s.raw());
        k += 1;
    }

    d.hash
}

#[test]
fn math_is_byte_identical_to_golden() {
    let digest = compute();
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden-math.txt");

    if std::env::var_os("ENGINE_GOLDEN_WRITE").is_some() {
        std::fs::write(path, format!("{digest:016x}\n")).expect("write golden file");
        eprintln!("wrote golden-math.txt: {digest:016x}");
        return;
    }

    let golden = std::fs::read_to_string(path)
        .expect("tests/golden-math.txt missing — run `just gen-golden`");
    let golden = u64::from_str_radix(golden.trim(), 16).expect("parse golden digest");
    assert_eq!(
        digest, golden,
        "engine-math determinism digest changed: {digest:016x} != golden {golden:016x}.\n\
         If this change was intentional, regenerate with `just gen-golden`."
    );
}

#[test]
fn digest_is_stable_within_a_run() {
    assert_eq!(compute(), compute());
}
