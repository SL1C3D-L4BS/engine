//! `engine-raster` — software rasterizer testbed, the rendering oracle.
//!
//! See ENGINE_SPECIFICATION_v2.0.md Part IX. Pure CPU Rust, std-only.
//! Produces pixel-accurate reference images against which the GPU
//! pipeline is regression-tested.
//!
//! Phase 5 design ADRs that bind this crate:
//!
//! - ADR-046 · Rasterizer testbed oracle · regression criteria and
//!   exception process (the 1/255 threshold, p99 ≤ 1% violation, sRGB-
//!   aware comparison, exception register).
//! - ADR-039 · Render-graph abstraction (the testbed implements the
//!   same `render_graph::Pass` interface so the CPU and GPU paths can
//!   be cross-checked).
//! - ADR-053 · Phase 5 PR 1.
//!
//! ## Phase 5 PR 1 status
//!
//! The substantive rasterizer ships: triangle rasterisation via the
//! edge-function method (Pineda 1988), a Z-buffer, perspective-correct
//! per-vertex attribute interpolation, sRGB-aware framebuffer output,
//! and the `Framebuffer` + `RenderTarget` types the oracle compares
//! against. Tile-parallel inner loop + the `std::simd` SIMD path are
//! Phase 5 PR 6 work (the milestone gate measures pacing; today the
//! rasterizer is single-threaded and uses scalar arithmetic). The
//! image-diff oracle (ADR-046) ships alongside in
//! `tests/raster_oracle.rs`.

pub mod framebuffer;
pub mod oracle;
pub mod rasterize;
pub mod sample;

pub use framebuffer::{Framebuffer, Rgba8};
pub use oracle::{ImageComparison, OracleVerdict, compare_images};
pub use rasterize::{Vertex, Viewport, clear, rasterize_triangle};
pub use sample::{GoldenScene, golden_triangle_scene};
