//! `engine-render` — two-track renderer — deferred PBR (3D) and 2D
//!
//! Level 2 crate. See ENGINE_SPECIFICATION_v2.0.md Part IV.1, Part IV.4
//! (Two-Track Pipeline), and Part IV.5 (Frame Pacing Contract).
//!
//! Phase 5 design ADRs that bind this crate before implementation:
//!
//! - ADR-039 · Render-graph abstraction (resource DAG, Track A/B
//!   compile-time selection, oracle-guarantee contract).
//! - ADR-040 · CSM cascade selection and atlas layout.
//! - ADR-041 · IBL · L2 SH probe generation and sampling.
//! - ADR-042 · TAA accumulation and rejection strategy.
//! - ADR-043 · Cluster lights · 16×9×24 binning.
//! - ADR-044 · Bindless texture heap allocation.
//! - ADR-045 · Texture compression fallback (BC7/BC5/BC4).
//! - ADR-046 · Rasterizer testbed oracle · regression criteria.
//! - ADR-047 · Frame Pacing CI gate.
//! - ADR-049 · `engine-gpu` owned wgpu wrapper (this crate consumes
//!   `engine_gpu`, never `wgpu` directly).
//! - ADR-053 · Phase 5 PR slicing (6-PR plan).
//!
//! ## Phase 5 PR 1 status
//!
//! The render-graph trait surface (this crate's `render_graph`
//! module) lands as part of Phase 5 PR 1 alongside the substantive
//! software rasterizer in `testbed/engine-raster`. Subsequent PRs
//! 2–6 wire concrete passes (depth pre-pass, GBuffer fill, CSM,
//! shading, TAA, upscale) into the graph.
//!
//! ## Phase 5 PR 2 status
//!
//! [`engine_gpu`] is now a direct dependency — the GPU-backed pass
//! types declared in PR 3+ name `engine_gpu::Device` / `Buffer` /
//! `Texture` / `BindlessHeap`, never `wgpu::*` (ADR-049 boundary).
//! No concrete pass is registered yet; PR 3 lands the first one
//! (deferred G-buffer + cluster lights + CSM).

pub mod render_graph;

pub use engine_gpu as gpu;
pub use render_graph::{
    Pass, PassContext, RenderGraph, Resource, ResourceId, ResourceKind, ResourceSet, Track,
};
