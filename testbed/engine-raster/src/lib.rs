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

pub mod cluster;
pub mod framebuffer;
pub mod oracle;
pub mod rasterize;
pub mod sample;
pub mod scene;
pub mod shading;
pub mod shadow;

pub use cluster::{
    CLUSTER_CELL_COUNT, CLUSTER_SLICES, CLUSTER_TILES_X, CLUSTER_TILES_Y, ClusterCell, ClusterGrid,
    MAX_LIGHTS_PER_CLUSTER, assign_lights, cell_index, cell_world_corners, cell_world_sphere,
    slice_of_view_z, slice_z,
};
pub use framebuffer::{Framebuffer, Rgba8};
pub use oracle::{ImageComparison, OracleVerdict, compare_images};
pub use rasterize::{Vertex, Viewport, clear, rasterize_triangle};
pub use sample::{
    GoldenScene, cluster_lights_scene, combined_deferred_scene, golden_triangle_scene,
    shadow_heavy_scene,
};
pub use scene::{Aabb, Camera, Frustum, Light, LightType, Material, MeshInstance, Plane};
pub use shading::{
    SurfaceFragment, accumulate_lighting, cook_torrance, screen_tile, view_space_depth,
};
pub use shadow::{
    ATLAS_DIM, CASCADE_DIM, CSM_CASCADES, Cascade, Cascades, PRACTICAL_SPLIT_LAMBDA, ShadowAtlas,
    atlas_origin, build_cascades, cascade_splits, cascade_view_projection, render_cascades,
    sample_shadow_pcf, select_cascade,
};
