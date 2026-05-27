//! Owned ONNX temporal upscaler binding stub. Active only when the
//! `ort-runtime` cargo feature is enabled. Per ADR-067.
//!
//! ## Scaffold status (Phase 6 PR 5.5)
//!
//! This module is the future home for the OwnedOnnxTemporal provider
//! impl. Today it is a documented stub — the real binding lands in a
//! follow-up PR that has the `ort` (ONNX Runtime) crate added as a
//! direct dependency + Git LFS configured for the bundled model.
//! The follow-up PR adds:
//!
//! - `ort` crate as an `engine-upscale-vendor` dep behind the
//!   `ort-runtime` feature. Default no-feature build stays
//!   ORT-binary-free per the engine's pre-PR-5.5 behaviour.
//! - `struct OwnedOnnxTemporalReal` that implements
//!   [`engine_render::UpscalerProvider`] by lazy-initializing an
//!   `ort::Session` on the first `upscale()` call (handles the
//!   ~200 ms session-build cost gracefully outside the frame budget).
//! - Backend cascade per `ort`'s defaults: CUDA → ROCm → DirectML
//!   (Windows) → CoreML (macOS) → CPU.
//! - Model bundling: `crates/engine-render/assets/onnx/
//!   temporal_upscaler_v1.onnx` (~3 MiB) tracked via Git LFS.
//!   Content-addressed via BLAKE3, verified at load time.
//! - ADR-051 deviation entry 4 (already landed in the PR-5.5
//!   addendum) becomes the canonical record.
//!
//! When the follow-up lands, this stub becomes the real binding and
//! `engine_render::upscale::OwnedOnnxTemporal::supports()` flips from
//! the Phase-6-PR-5 stub `false` to a runtime probe of the ORT
//! session-init success (per ADR-067 §6: "universal coverage" means
//! `supports() = true` whenever the session can initialize).

use engine_render::UpscalerKind;

/// Identifies this provider's vendor at the trait boundary.
pub const KIND: UpscalerKind = UpscalerKind::OwnedOnnx;

/// Stub probe — always returns false in the PR-5.5 scaffold; the
/// real implementation lands when `ort` dep + bundled model land.
pub fn supports_stub() -> bool {
    false
}

/// Pinned model filename. The bundled asset lives at
/// `crates/engine-render/assets/onnx/<MODEL_FILENAME>` once the
/// follow-up adds Git LFS tracking.
pub const MODEL_FILENAME: &str = "temporal_upscaler_v1.onnx";
