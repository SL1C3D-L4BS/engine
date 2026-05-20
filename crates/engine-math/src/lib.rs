//! `engine-math` — deterministic math: vectors, matrices, quaternions,
//! fixed-point, and transcendentals.
//!
//! Level 0 crate (no engine dependencies). See `ENGINE_SPECIFICATION_v2.0.md`
//! Part IV.1.
//!
//! # Determinism
//!
//! This crate is the foundation of the Determinism Contract (spec IV.2,
//! ADR-013). Every operation here is built from the IEEE-754 operations that
//! are *correctly rounded* and therefore identical on every conforming
//! platform — `+ - * /`, `sqrt`, and the exact integral operations. The crate
//! never issues a fused multiply-add, and it never calls the system math
//! library; transcendentals are owned polynomial approximations in
//! [`transcendental`]. See ADR-023 for the full rationale.
//!
//! The naming note: the spec's `f32_det` / `f64_det` wrappers are spelled
//! [`F32Det`] / [`F64Det`] here to satisfy Rust's type-naming conventions.

pub mod fixed;
pub mod mat;
pub mod quat;
pub mod scalar;
pub mod transcendental;
pub mod vec;

// Phase 1 SIMD wrapper. Private — the public surface stays scalar; vec.rs
// and mat.rs use this internally to gain SIMD throughput without changing
// the API or the deterministic reduction order (ADR-027).
mod simd;

pub use fixed::{I16F16, I32F32};
pub use mat::{Mat3, Mat4};
pub use quat::Quat;
pub use scalar::{
    F32Det, F64Det, clamp, clamp_f64, lerp, lerp_f64, saturate, saturate_f64, sign, sign_f64,
    smoothstep, smoothstep_f64,
};
pub use vec::{Vec2, Vec3, Vec4, vec2, vec3, vec4};
