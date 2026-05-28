//! 3D Gaussian Splatting · ADR-077 architecture, ADR-078 asset format.
//!
//! This crate implements the engine's 3DGS renderer per the Kerbl et al.
//! 2023 SIGGRAPH paper plus the Khronos KHR_gaussian_splatting glTF
//! extension. Level-2 in the workspace stack: depends on `engine-math`,
//! `engine-platform`, `engine-asset`, and `blake3`. No `engine-render`,
//! `engine-script`, or any upper-layer dep.
//!
//! ## Public surface
//!
//! - [`SplatCloud`] — SoA storage for an N-splat point cloud
//!   (positions / scales / rotations / colors / opacities / optional
//!   spherical-harmonics per ADR-077 §2).
//! - [`SplatCloudBuilder`] — owned-mutation constructor used by the
//!   asset-decoding path.
//! - [`asset`] — ESPL binary encode/decode per ADR-078.
//! - [`sort`] — parallel radix sort by camera-space depth
//!   (CPU + GPU reference implementations).
//! - [`composite`] — render-graph pass surface for back-to-front
//!   alpha-blend composition.
//! - [`gltf_ext`] — glTF KHR_gaussian_splatting extension reader
//!   (used only by `tools/engine-splat-import/`; the engine binary
//!   never re-parses glTF at runtime).
//!
//! ## Crate layering
//!
//! `engine-splatting` sits at Level 2 alongside `engine-render`. The
//! 3DGS path is *additive* to the deferred-PBR Track-A renderer: the
//! composite pass runs after `IblPass` + `LightingPass` and writes
//! into the scene-color target, which TAA then reads.

#![deny(missing_docs)]

pub mod asset;
pub mod cloud;
pub mod composite;
pub mod contracts;
pub mod gltf_ext;
pub mod sort;

pub use asset::{SplatCloudMeta, decode, encode};
pub use cloud::{SplatCloud, SplatCloudBuilder};
pub use composite::SplatCompositePass;
pub use sort::cpu as cpu_sort;
