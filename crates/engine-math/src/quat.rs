//! Unit quaternion type for 3D rotations.
//!
//! All trigonometry routes through the owned [`crate::transcendental`] module,
//! so quaternion construction and interpolation are deterministic (ADR-023).

use crate::transcendental::{acos_f32, cos_f32, sin_f32};
use crate::vec::Vec3;
use core::ops::Mul;

/// A quaternion `(x, y, z, w)` representing a rotation when unit-length.
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct Quat {
    /// Imaginary `i` component.
    pub x: f32,
    /// Imaginary `j` component.
    pub y: f32,
    /// Imaginary `k` component.
    pub z: f32,
    /// Real component.
    pub w: f32,
}

impl Default for Quat {
    #[inline]
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Quat {
    /// The identity rotation.
    pub const IDENTITY: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 0.0,
        w: 1.0,
    };

    /// Constructs a quaternion from raw components.
    #[inline]
    pub const fn new(x: f32, y: f32, z: f32, w: f32) -> Self {
        Self { x, y, z, w }
    }

    /// Builds a rotation of `angle` radians about `axis`.
    ///
    /// `axis` is normalized internally; a zero-length axis yields the identity.
    pub fn from_axis_angle(axis: Vec3, angle: f32) -> Self {
        let axis = axis.normalize_or_zero();
        if axis == Vec3::ZERO {
            return Self::IDENTITY;
        }
        let half = angle * 0.5;
        let s = sin_f32(half);
        Self {
            x: axis.x * s,
            y: axis.y * s,
            z: axis.z * s,
            w: cos_f32(half),
        }
    }

    /// Builds a rotation from Euler angles (radians).
    ///
    /// Application order is roll (`z`), then pitch (`x`), then yaw (`y`).
    pub fn from_euler(x_pitch: f32, y_yaw: f32, z_roll: f32) -> Self {
        let qx = Self::from_axis_angle(Vec3::X, x_pitch);
        let qy = Self::from_axis_angle(Vec3::Y, y_yaw);
        let qz = Self::from_axis_angle(Vec3::Z, z_roll);
        qy * (qx * qz)
    }

    /// The conjugate; for a unit quaternion this is the inverse rotation.
    #[inline]
    pub fn conjugate(self) -> Self {
        Self {
            x: -self.x,
            y: -self.y,
            z: -self.z,
            w: self.w,
        }
    }

    /// Quaternion dot product.
    #[inline]
    pub fn dot(self, o: Self) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z + self.w * o.w
    }

    /// Euclidean length.
    #[inline]
    pub fn length(self) -> f32 {
        self.dot(self).sqrt()
    }

    /// Returns the unit quaternion, or [`Quat::IDENTITY`] if degenerate.
    #[inline]
    pub fn normalize_or_identity(self) -> Self {
        let len = self.length();
        if len > 0.0 {
            Self {
                x: self.x / len,
                y: self.y / len,
                z: self.z / len,
                w: self.w / len,
            }
        } else {
            Self::IDENTITY
        }
    }

    /// Rotates a vector by this quaternion.
    ///
    /// Uses the standard `v + 2w(u×v) + 2(u×(u×v))` identity, where `u` is the
    /// imaginary part — cheaper than `q * v * q⁻¹` and free of fused
    /// multiply-adds.
    pub fn rotate(self, v: Vec3) -> Vec3 {
        let u = Vec3::new(self.x, self.y, self.z);
        let t = u.cross(v) * 2.0;
        v + t * self.w + u.cross(t)
    }

    /// Spherical linear interpolation toward `o` by `t`.
    pub fn slerp(self, mut o: Self, t: f32) -> Self {
        let mut d = self.dot(o);
        // Take the shorter arc.
        if d < 0.0 {
            o = Self {
                x: -o.x,
                y: -o.y,
                z: -o.z,
                w: -o.w,
            };
            d = -d;
        }
        // Near-parallel: fall back to normalized linear interpolation.
        if d > 0.9995 {
            return Self {
                x: self.x + (o.x - self.x) * t,
                y: self.y + (o.y - self.y) * t,
                z: self.z + (o.z - self.z) * t,
                w: self.w + (o.w - self.w) * t,
            }
            .normalize_or_identity();
        }
        let theta = acos_f32(d);
        let sin_theta = sin_f32(theta);
        let a = sin_f32(theta * (1.0 - t)) / sin_theta;
        let b = sin_f32(theta * t) / sin_theta;
        Self {
            x: self.x * a + o.x * b,
            y: self.y * a + o.y * b,
            z: self.z * a + o.z * b,
            w: self.w * a + o.w * b,
        }
    }
}

impl Mul for Quat {
    type Output = Self;

    /// Hamilton product: the rotation `self` applied after `rhs`.
    #[inline]
    fn mul(self, b: Self) -> Self {
        let a = self;
        Self {
            w: a.w * b.w - a.x * b.x - a.y * b.y - a.z * b.z,
            x: a.w * b.x + a.x * b.w + a.y * b.z - a.z * b.y,
            y: a.w * b.y - a.x * b.z + a.y * b.w + a.z * b.x,
            z: a.w * b.z + a.x * b.y - a.y * b.x + a.z * b.w,
        }
    }
}

// Layout invariants (Phase 1 cache observatory). A quaternion is four `f32`s
// in `(x, y, z, w)` order, matching glTF and the on-disk asset format.
const _: () = assert!(core::mem::size_of::<Quat>() == 16);
const _: () = assert!(core::mem::align_of::<Quat>() == 4);

#[cfg(test)]
mod tests {
    use super::*;
    use core::f32::consts::PI;

    fn approx(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-4
    }

    #[test]
    fn identity_rotates_nothing() {
        let v = Vec3::new(1.0, 2.0, 3.0);
        assert!(approx(Quat::IDENTITY.rotate(v), v));
    }

    #[test]
    fn quarter_turn_about_z() {
        let q = Quat::from_axis_angle(Vec3::Z, PI / 2.0);
        assert!(approx(q.rotate(Vec3::X), Vec3::Y));
    }

    #[test]
    fn conjugate_undoes_rotation() {
        let q = Quat::from_axis_angle(Vec3::new(1.0, 1.0, 0.0), 0.7);
        let v = Vec3::new(2.0, -1.0, 0.5);
        assert!(approx(q.conjugate().rotate(q.rotate(v)), v));
    }

    #[test]
    fn slerp_endpoints() {
        let a = Quat::IDENTITY;
        let b = Quat::from_axis_angle(Vec3::Y, 1.2);
        assert!(approx(a.slerp(b, 0.0).rotate(Vec3::X), a.rotate(Vec3::X)));
        assert!(approx(a.slerp(b, 1.0).rotate(Vec3::X), b.rotate(Vec3::X)));
    }
}
