//! Frozen scalar reference for the Phase 1 SIMD parity oracle.
//!
//! This file is *not* `mod`'d into `lib.rs`. It lives under `src/` so it is
//! formatted and linted, but it is `#[path]`-included by
//! `tests/simd_parity.rs` only. The implementations here mirror the original
//! pre-SIMD scalar math byte-for-byte — they exist exclusively so the parity
//! test can assert that the type's own methods produce the same bits after
//! SIMD replacement (ADR-027). Do not edit unless the parity oracle itself
//! changes.
//!
//! Reduction orders here are load-bearing. The SIMD replacements must
//! preserve them exactly:
//!
//! - `vec3_dot`     → `x*ox + y*oy + z*oz` (left-to-right)
//! - `vec4_dot`     → `((x*ox + y*oy) + z*oz) + w*ow`
//! - `vec3_cross`   → component-wise subtraction, original ordering
//! - `mat3_mul`     → accumulate `k` ascending, `((0 + p0) + p1) + p2` per cell
//! - `mat4_mul`     → accumulate `k` ascending, `((0 + p0) + p1) + p2 + p3`
//! - `mat4_mul_vec` → `m[0]*v.x + m[4]*v.y + m[8]*v.z + m[12]*v.w` per row

#![allow(dead_code)] // included via #[path] from an integration test; not
// every helper has a caller at every revision.

// `#[path]`-included from `tests/simd_parity.rs`, so `crate` resolves to the
// test crate, not `engine-math`. Reach back into `engine-math` by name.
use engine_math::mat::{Mat3, Mat4};
use engine_math::vec::{Vec2, Vec3, Vec4};

// --- Vec arithmetic ---------------------------------------------------

pub fn vec2_add(a: Vec2, b: Vec2) -> Vec2 {
    Vec2 {
        x: a.x + b.x,
        y: a.y + b.y,
    }
}

pub fn vec3_add(a: Vec3, b: Vec3) -> Vec3 {
    Vec3 {
        x: a.x + b.x,
        y: a.y + b.y,
        z: a.z + b.z,
    }
}

pub fn vec4_add(a: Vec4, b: Vec4) -> Vec4 {
    Vec4 {
        x: a.x + b.x,
        y: a.y + b.y,
        z: a.z + b.z,
        w: a.w + b.w,
    }
}

pub fn vec3_sub(a: Vec3, b: Vec3) -> Vec3 {
    Vec3 {
        x: a.x - b.x,
        y: a.y - b.y,
        z: a.z - b.z,
    }
}

pub fn vec4_sub(a: Vec4, b: Vec4) -> Vec4 {
    Vec4 {
        x: a.x - b.x,
        y: a.y - b.y,
        z: a.z - b.z,
        w: a.w - b.w,
    }
}

pub fn vec3_mul_scalar(a: Vec3, s: f32) -> Vec3 {
    Vec3 {
        x: a.x * s,
        y: a.y * s,
        z: a.z * s,
    }
}

pub fn vec4_mul_scalar(a: Vec4, s: f32) -> Vec4 {
    Vec4 {
        x: a.x * s,
        y: a.y * s,
        z: a.z * s,
        w: a.w * s,
    }
}

pub fn vec3_mul(a: Vec3, b: Vec3) -> Vec3 {
    Vec3 {
        x: a.x * b.x,
        y: a.y * b.y,
        z: a.z * b.z,
    }
}

pub fn vec4_mul(a: Vec4, b: Vec4) -> Vec4 {
    Vec4 {
        x: a.x * b.x,
        y: a.y * b.y,
        z: a.z * b.z,
        w: a.w * b.w,
    }
}

pub fn vec3_div_scalar(a: Vec3, s: f32) -> Vec3 {
    Vec3 {
        x: a.x / s,
        y: a.y / s,
        z: a.z / s,
    }
}

pub fn vec4_div_scalar(a: Vec4, s: f32) -> Vec4 {
    Vec4 {
        x: a.x / s,
        y: a.y / s,
        z: a.z / s,
        w: a.w / s,
    }
}

pub fn vec3_neg(a: Vec3) -> Vec3 {
    Vec3 {
        x: -a.x,
        y: -a.y,
        z: -a.z,
    }
}

pub fn vec4_neg(a: Vec4) -> Vec4 {
    Vec4 {
        x: -a.x,
        y: -a.y,
        z: -a.z,
        w: -a.w,
    }
}

// --- dot / cross ------------------------------------------------------

pub fn vec3_dot(a: Vec3, b: Vec3) -> f32 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

pub fn vec4_dot(a: Vec4, b: Vec4) -> f32 {
    a.x * b.x + a.y * b.y + a.z * b.z + a.w * b.w
}

pub fn vec3_cross(a: Vec3, b: Vec3) -> Vec3 {
    Vec3 {
        x: a.y * b.z - a.z * b.y,
        y: a.z * b.x - a.x * b.z,
        z: a.x * b.y - a.y * b.x,
    }
}

// --- Mat3/Mat4 multiplies --------------------------------------------

pub fn mat3_mul(lhs: Mat3, rhs: Mat3) -> Mat3 {
    let a = lhs.to_cols_array();
    let b = rhs.to_cols_array();
    let mut out = [0.0f32; 9];
    for c in 0..3 {
        for r in 0..3 {
            let mut s = 0.0;
            for k in 0..3 {
                s += a[k * 3 + r] * b[c * 3 + k];
            }
            out[c * 3 + r] = s;
        }
    }
    Mat3::from_cols_array(out)
}

pub fn mat4_mul(lhs: Mat4, rhs: Mat4) -> Mat4 {
    let a = lhs.to_cols_array();
    let b = rhs.to_cols_array();
    let mut out = [0.0f32; 16];
    for c in 0..4 {
        for r in 0..4 {
            let mut s = 0.0;
            for k in 0..4 {
                s += a[k * 4 + r] * b[c * 4 + k];
            }
            out[c * 4 + r] = s;
        }
    }
    Mat4::from_cols_array(out)
}

pub fn mat4_mul_vec(lhs: Mat4, v: Vec4) -> Vec4 {
    let m = lhs.to_cols_array();
    Vec4::new(
        m[0] * v.x + m[4] * v.y + m[8] * v.z + m[12] * v.w,
        m[1] * v.x + m[5] * v.y + m[9] * v.z + m[13] * v.w,
        m[2] * v.x + m[6] * v.y + m[10] * v.z + m[14] * v.w,
        m[3] * v.x + m[7] * v.y + m[11] * v.z + m[15] * v.w,
    )
}
