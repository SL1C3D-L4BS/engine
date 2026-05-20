//! Column-major `f32` matrix types: [`Mat3`] and [`Mat4`].
//!
//! Storage is column-major (the convention shared by GLSL, Slang, and wgpu),
//! laid out flat so that a column `c`, row `r` element lives at index
//! `c * dim + r`. All arithmetic is plain IEEE-754 (no fused multiply-add),
//! hence deterministic (ADR-023).

use crate::quat::Quat;
use crate::simd::Simd4f;
use crate::vec::{Vec3, Vec4};
use core::ops::Mul;

/// A 3×3 column-major matrix (rotation / scale, normal matrices).
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct Mat3 {
    m: [f32; 9],
}

/// A 4×4 column-major matrix (affine and projective transforms).
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct Mat4 {
    m: [f32; 16],
}

impl Default for Mat3 {
    #[inline]
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Default for Mat4 {
    #[inline]
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Mat3 {
    /// The identity matrix.
    pub const IDENTITY: Self = Self {
        m: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
    };

    /// Builds a matrix from a flat column-major array.
    #[inline]
    pub const fn from_cols_array(m: [f32; 9]) -> Self {
        Self { m }
    }

    /// Returns the flat column-major array.
    #[inline]
    pub const fn to_cols_array(self) -> [f32; 9] {
        self.m
    }

    /// The rotation matrix equivalent to the given unit quaternion.
    pub fn from_quat(q: Quat) -> Self {
        let Quat { x, y, z, w } = q;
        let (xx, yy, zz) = (x * x, y * y, z * z);
        let (xy, xz, yz) = (x * y, x * z, y * z);
        let (wx, wy, wz) = (w * x, w * y, w * z);
        // Column-major: index = col * 3 + row.
        Self {
            m: [
                1.0 - 2.0 * (yy + zz),
                2.0 * (xy + wz),
                2.0 * (xz - wy),
                2.0 * (xy - wz),
                1.0 - 2.0 * (xx + zz),
                2.0 * (yz + wx),
                2.0 * (xz + wy),
                2.0 * (yz - wx),
                1.0 - 2.0 * (xx + yy),
            ],
        }
    }

    /// The transpose.
    pub fn transpose(self) -> Self {
        let m = &self.m;
        Self {
            m: [m[0], m[3], m[6], m[1], m[4], m[7], m[2], m[5], m[8]],
        }
    }

    /// The determinant.
    pub fn determinant(self) -> f32 {
        let m = &self.m;
        m[0] * (m[4] * m[8] - m[7] * m[5]) - m[3] * (m[1] * m[8] - m[7] * m[2])
            + m[6] * (m[1] * m[5] - m[4] * m[2])
    }

    /// The inverse, or `None` when the matrix is singular.
    pub fn inverse(self) -> Option<Self> {
        let det = self.determinant();
        if det == 0.0 {
            return None;
        }
        let inv = 1.0 / det;
        let m = &self.m;
        // Cofactors, written transposed to form the adjugate directly.
        Some(Self {
            m: [
                (m[4] * m[8] - m[7] * m[5]) * inv,
                (m[7] * m[2] - m[1] * m[8]) * inv,
                (m[1] * m[5] - m[4] * m[2]) * inv,
                (m[6] * m[5] - m[3] * m[8]) * inv,
                (m[0] * m[8] - m[6] * m[2]) * inv,
                (m[3] * m[2] - m[0] * m[5]) * inv,
                (m[3] * m[7] - m[6] * m[4]) * inv,
                (m[6] * m[1] - m[0] * m[7]) * inv,
                (m[0] * m[4] - m[3] * m[1]) * inv,
            ],
        })
    }
}

impl Mat4 {
    /// The identity matrix.
    pub const IDENTITY: Self = Self {
        m: [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ],
    };

    /// Builds a matrix from a flat column-major array.
    #[inline]
    pub const fn from_cols_array(m: [f32; 16]) -> Self {
        Self { m }
    }

    /// Returns the flat column-major array.
    #[inline]
    pub const fn to_cols_array(self) -> [f32; 16] {
        self.m
    }

    /// Returns column `c` (`0..4`) as a [`Vec4`].
    #[inline]
    pub fn col(self, c: usize) -> Vec4 {
        let b = c * 4;
        Vec4::new(self.m[b], self.m[b + 1], self.m[b + 2], self.m[b + 3])
    }

    /// A pure translation matrix.
    pub fn from_translation(t: Vec3) -> Self {
        let mut m = Self::IDENTITY;
        m.m[12] = t.x;
        m.m[13] = t.y;
        m.m[14] = t.z;
        m
    }

    /// A pure (non-uniform) scale matrix.
    pub fn from_scale(s: Vec3) -> Self {
        let mut m = Self::IDENTITY;
        m.m[0] = s.x;
        m.m[5] = s.y;
        m.m[10] = s.z;
        m
    }

    /// A pure rotation matrix from a unit quaternion.
    pub fn from_quat(q: Quat) -> Self {
        let r = Mat3::from_quat(q).to_cols_array();
        Self {
            m: [
                r[0], r[1], r[2], 0.0, r[3], r[4], r[5], 0.0, r[6], r[7], r[8], 0.0, 0.0, 0.0, 0.0,
                1.0,
            ],
        }
    }

    /// Composes a translation·rotation·scale transform (applied scale-first).
    pub fn from_trs(translation: Vec3, rotation: Quat, scale: Vec3) -> Self {
        Self::from_translation(translation) * Self::from_quat(rotation) * Self::from_scale(scale)
    }

    /// The transpose.
    pub fn transpose(self) -> Self {
        let m = &self.m;
        Self {
            m: [
                m[0], m[4], m[8], m[12], m[1], m[5], m[9], m[13], m[2], m[6], m[10], m[14], m[3],
                m[7], m[11], m[15],
            ],
        }
    }

    /// The inverse, or `None` when the matrix is singular.
    ///
    /// This is the cofactor-expansion inverse (the classic MESA routine),
    /// valid for any invertible 4×4 matrix.
    pub fn inverse(self) -> Option<Self> {
        let m = &self.m;
        let mut inv = [0.0f32; 16];

        inv[0] = m[5] * m[10] * m[15] - m[5] * m[11] * m[14] - m[9] * m[6] * m[15]
            + m[9] * m[7] * m[14]
            + m[13] * m[6] * m[11]
            - m[13] * m[7] * m[10];
        inv[4] = -m[4] * m[10] * m[15] + m[4] * m[11] * m[14] + m[8] * m[6] * m[15]
            - m[8] * m[7] * m[14]
            - m[12] * m[6] * m[11]
            + m[12] * m[7] * m[10];
        inv[8] = m[4] * m[9] * m[15] - m[4] * m[11] * m[13] - m[8] * m[5] * m[15]
            + m[8] * m[7] * m[13]
            + m[12] * m[5] * m[11]
            - m[12] * m[7] * m[9];
        inv[12] = -m[4] * m[9] * m[14] + m[4] * m[10] * m[13] + m[8] * m[5] * m[14]
            - m[8] * m[6] * m[13]
            - m[12] * m[5] * m[10]
            + m[12] * m[6] * m[9];
        inv[1] = -m[1] * m[10] * m[15] + m[1] * m[11] * m[14] + m[9] * m[2] * m[15]
            - m[9] * m[3] * m[14]
            - m[13] * m[2] * m[11]
            + m[13] * m[3] * m[10];
        inv[5] = m[0] * m[10] * m[15] - m[0] * m[11] * m[14] - m[8] * m[2] * m[15]
            + m[8] * m[3] * m[14]
            + m[12] * m[2] * m[11]
            - m[12] * m[3] * m[10];
        inv[9] = -m[0] * m[9] * m[15] + m[0] * m[11] * m[13] + m[8] * m[1] * m[15]
            - m[8] * m[3] * m[13]
            - m[12] * m[1] * m[11]
            + m[12] * m[3] * m[9];
        inv[13] = m[0] * m[9] * m[14] - m[0] * m[10] * m[13] - m[8] * m[1] * m[14]
            + m[8] * m[2] * m[13]
            + m[12] * m[1] * m[10]
            - m[12] * m[2] * m[9];
        inv[2] = m[1] * m[6] * m[15] - m[1] * m[7] * m[14] - m[5] * m[2] * m[15]
            + m[5] * m[3] * m[14]
            + m[13] * m[2] * m[7]
            - m[13] * m[3] * m[6];
        inv[6] = -m[0] * m[6] * m[15] + m[0] * m[7] * m[14] + m[4] * m[2] * m[15]
            - m[4] * m[3] * m[14]
            - m[12] * m[2] * m[7]
            + m[12] * m[3] * m[6];
        inv[10] = m[0] * m[5] * m[15] - m[0] * m[7] * m[13] - m[4] * m[1] * m[15]
            + m[4] * m[3] * m[13]
            + m[12] * m[1] * m[7]
            - m[12] * m[3] * m[5];
        inv[14] = -m[0] * m[5] * m[14] + m[0] * m[6] * m[13] + m[4] * m[1] * m[14]
            - m[4] * m[2] * m[13]
            - m[12] * m[1] * m[6]
            + m[12] * m[2] * m[5];
        inv[3] = -m[1] * m[6] * m[11] + m[1] * m[7] * m[10] + m[5] * m[2] * m[11]
            - m[5] * m[3] * m[10]
            - m[9] * m[2] * m[7]
            + m[9] * m[3] * m[6];
        inv[7] = m[0] * m[6] * m[11] - m[0] * m[7] * m[10] - m[4] * m[2] * m[11]
            + m[4] * m[3] * m[10]
            + m[8] * m[2] * m[7]
            - m[8] * m[3] * m[6];
        inv[11] = -m[0] * m[5] * m[11] + m[0] * m[7] * m[9] + m[4] * m[1] * m[11]
            - m[4] * m[3] * m[9]
            - m[8] * m[1] * m[7]
            + m[8] * m[3] * m[5];
        inv[15] = m[0] * m[5] * m[10] - m[0] * m[6] * m[9] - m[4] * m[1] * m[10]
            + m[4] * m[2] * m[9]
            + m[8] * m[1] * m[6]
            - m[8] * m[2] * m[5];

        let det = m[0] * inv[0] + m[1] * inv[4] + m[2] * inv[8] + m[3] * inv[12];
        if det == 0.0 {
            return None;
        }
        let det = 1.0 / det;
        for v in &mut inv {
            *v *= det;
        }
        Some(Self { m: inv })
    }

    /// Transforms a point (implicit `w = 1`, with perspective divide).
    pub fn transform_point3(self, p: Vec3) -> Vec3 {
        let v = self * p.extend(1.0);
        if v.w != 0.0 {
            Vec3::new(v.x / v.w, v.y / v.w, v.z / v.w)
        } else {
            v.truncate()
        }
    }

    /// Transforms a direction (implicit `w = 0`, ignores translation).
    pub fn transform_vector3(self, v: Vec3) -> Vec3 {
        (self * v.extend(0.0)).truncate()
    }

    /// Right-handed perspective projection with a `[0, 1]` depth range
    /// (the wgpu / Vulkan convention). `fov_y` is in radians.
    pub fn perspective_rh(fov_y: f32, aspect: f32, near: f32, far: f32) -> Self {
        let f = 1.0 / crate::transcendental::tan_f32(fov_y * 0.5);
        let mut m = [0.0f32; 16];
        m[0] = f / aspect;
        m[5] = f;
        m[10] = far / (near - far);
        m[11] = -1.0;
        m[14] = (far * near) / (near - far);
        Self { m }
    }

    /// Right-handed orthographic projection with a `[0, 1]` depth range.
    pub fn orthographic_rh(
        left: f32,
        right: f32,
        bottom: f32,
        top: f32,
        near: f32,
        far: f32,
    ) -> Self {
        let mut m = [0.0f32; 16];
        m[0] = 2.0 / (right - left);
        m[5] = 2.0 / (top - bottom);
        m[10] = 1.0 / (near - far);
        m[12] = (right + left) / (left - right);
        m[13] = (top + bottom) / (bottom - top);
        m[14] = near / (near - far);
        m[15] = 1.0;
        Self { m }
    }
}

impl Mul for Mat3 {
    type Output = Self;

    /// Column-at-a-time SIMD multiply.
    ///
    /// Each output column is `sum over k of lhs.col(k) * rhs[c][k]`, with the
    /// sum accumulated in **ascending k order** — the same reduction order
    /// the pre-SIMD code used. Lane 3 of every loaded column is zero and
    /// stays zero, so it never contaminates a real cell. The Phase 1 parity
    /// oracle locks this in (ADR-027).
    fn mul(self, rhs: Self) -> Self {
        let a = &self.m;
        let b = &rhs.m;
        let col_a = [
            Simd4f::new(a[0], a[1], a[2], 0.0),
            Simd4f::new(a[3], a[4], a[5], 0.0),
            Simd4f::new(a[6], a[7], a[8], 0.0),
        ];
        let mut out = [0.0f32; 9];
        for c in 0..3 {
            let mut col = Simd4f::splat(0.0);
            for k in 0..3 {
                col = col.add(col_a[k].mul(Simd4f::splat(b[c * 3 + k])));
            }
            let [x, y, z, _] = col.to_array();
            out[c * 3] = x;
            out[c * 3 + 1] = y;
            out[c * 3 + 2] = z;
        }
        Self { m: out }
    }
}

impl Mul<Vec3> for Mat3 {
    type Output = Vec3;

    fn mul(self, v: Vec3) -> Vec3 {
        let m = &self.m;
        Vec3::new(
            m[0] * v.x + m[3] * v.y + m[6] * v.z,
            m[1] * v.x + m[4] * v.y + m[7] * v.z,
            m[2] * v.x + m[5] * v.y + m[8] * v.z,
        )
    }
}

impl Mul for Mat4 {
    type Output = Self;

    /// Column-at-a-time SIMD multiply.
    ///
    /// Each output column is `sum over k of lhs.col(k) * rhs[c][k]`. The
    /// sum runs over `k` in ascending order; the four cells of one column
    /// are accumulated in parallel via element-wise SIMD `add`, so the
    /// per-cell reduction order is `((0 + p0) + p1) + p2 + p3` — identical
    /// to the pre-SIMD scalar implementation (Phase 1 parity oracle).
    fn mul(self, rhs: Self) -> Self {
        let a = &self.m;
        let b = &rhs.m;
        let col_a = [
            Simd4f::new(a[0], a[1], a[2], a[3]),
            Simd4f::new(a[4], a[5], a[6], a[7]),
            Simd4f::new(a[8], a[9], a[10], a[11]),
            Simd4f::new(a[12], a[13], a[14], a[15]),
        ];
        let mut out = [0.0f32; 16];
        for c in 0..4 {
            let mut col = Simd4f::splat(0.0);
            for k in 0..4 {
                col = col.add(col_a[k].mul(Simd4f::splat(b[c * 4 + k])));
            }
            let [x, y, z, w] = col.to_array();
            out[c * 4] = x;
            out[c * 4 + 1] = y;
            out[c * 4 + 2] = z;
            out[c * 4 + 3] = w;
        }
        Self { m: out }
    }
}

impl Mul<Vec4> for Mat4 {
    type Output = Vec4;

    /// Column-broadcast SIMD `Mat4 * Vec4`.
    ///
    /// `out = col(0)*splat(v.x) + col(1)*splat(v.y) + col(2)*splat(v.z) +
    /// col(3)*splat(v.w)`, accumulated left-to-right. Per row that expands
    /// to `m[r]*v.x + m[r+4]*v.y + m[r+8]*v.z + m[r+12]*v.w` — the exact
    /// scalar reduction order.
    fn mul(self, v: Vec4) -> Vec4 {
        let m = &self.m;
        let c0 = Simd4f::new(m[0], m[1], m[2], m[3]);
        let c1 = Simd4f::new(m[4], m[5], m[6], m[7]);
        let c2 = Simd4f::new(m[8], m[9], m[10], m[11]);
        let c3 = Simd4f::new(m[12], m[13], m[14], m[15]);
        let acc = c0
            .mul(Simd4f::splat(v.x))
            .add(c1.mul(Simd4f::splat(v.y)))
            .add(c2.mul(Simd4f::splat(v.z)))
            .add(c3.mul(Simd4f::splat(v.w)));
        Vec4::from_simd(acc)
    }
}

// Layout invariants (Phase 1 cache observatory). A `Mat4` fits in exactly one
// 64-byte cache line — the renderer's per-instance transform array relies on
// this so a single line fetch loads a whole matrix. `Mat3` is 36 bytes
// (9 × f32); we assert that explicitly to catch accidental padding.
const _: () = assert!(core::mem::size_of::<Mat3>() == 36);
const _: () = assert!(core::mem::align_of::<Mat3>() == 4);
const _: () = assert!(core::mem::size_of::<Mat4>() == 64);
const _: () = assert!(core::mem::align_of::<Mat4>() == 4);

#[cfg(test)]
mod tests {
    use super::*;
    use core::f32::consts::PI;

    fn approx4(a: Mat4, b: Mat4) -> bool {
        a.to_cols_array()
            .iter()
            .zip(b.to_cols_array().iter())
            .all(|(x, y)| (x - y).abs() < 1e-4)
    }

    #[test]
    fn identity_is_multiplicative_unit() {
        let t = Mat4::from_translation(Vec3::new(1.0, 2.0, 3.0));
        assert!(approx4(t * Mat4::IDENTITY, t));
        assert!(approx4(Mat4::IDENTITY * t, t));
    }

    #[test]
    fn translation_moves_points() {
        let t = Mat4::from_translation(Vec3::new(5.0, 0.0, 0.0));
        assert_eq!(t.transform_point3(Vec3::ZERO), Vec3::new(5.0, 0.0, 0.0));
        assert_eq!(t.transform_vector3(Vec3::X), Vec3::X);
    }

    #[test]
    fn inverse_round_trips() {
        let m = Mat4::from_trs(
            Vec3::new(1.0, -2.0, 3.0),
            Quat::from_axis_angle(Vec3::Y, 0.6),
            Vec3::new(2.0, 2.0, 2.0),
        );
        let inv = m.inverse().expect("invertible");
        assert!(approx4(m * inv, Mat4::IDENTITY));
    }

    #[test]
    fn mat3_inverse_round_trips() {
        let r = Mat3::from_quat(Quat::from_axis_angle(Vec3::Z, PI / 3.0));
        let inv = r.inverse().expect("rotation is invertible");
        let prod = r * inv;
        assert!(
            prod.to_cols_array()
                .iter()
                .zip(Mat3::IDENTITY.to_cols_array().iter())
                .all(|(x, y)| (x - y).abs() < 1e-4)
        );
    }

    #[test]
    fn projections_are_finite() {
        let p = Mat4::perspective_rh(PI / 3.0, 16.0 / 9.0, 0.1, 100.0);
        let o = Mat4::orthographic_rh(-1.0, 1.0, -1.0, 1.0, 0.1, 100.0);
        assert!(p.to_cols_array().iter().all(|v| v.is_finite()));
        assert!(o.to_cols_array().iter().all(|v| v.is_finite()));
    }
}
