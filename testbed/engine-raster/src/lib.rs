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
//!
//! ## Phase 5 PR 4 status
//!
//! The CPU oracle gains IBL (ADR-041) and the post-FX chain (ADR-042):
//!
//! - [`ibl`] — L2 SH probe storage + 8-neighbour trilinear sampling +
//!   Ramamoorthi-Hanrahan closed-form verification. Probes obey the
//!   `MAX_PROBES = 128` cap and `PROBE_CELL_SIZE = 4 m` default.
//! - [`post_fx`] — Halton (2, 3) period-8 jitter, YCgCo neighbourhood-
//!   clip TAA with motion-vector reprojection + disocclusion mask,
//!   8-tap SSAO, soft-knee bloom extract + composite, ACES filmic
//!   tonemap. Spec post-chain order:
//!   `SSAO → TAA → Bloom → Tonemap → (grade) → Upscale`.
//!
//! ## Phase 5 PR 5 status
//!
//! [`upscale`] — bilinear upscale CPU reference for ADR-005's
//! `Owned::Bilinear` placeholder. Pure CPU `+ − × ÷`, deterministic
//! per ADR-013. The frame-pacing milestone bench
//! (`bin/engine-bench-frame-pacing/`) is the integration consumer.

pub mod cluster;
pub mod framebuffer;
pub mod ibl;
pub mod oracle;
pub mod post_fx;
pub mod rasterize;
pub mod sample;
pub mod scene;
pub mod shading;
pub mod shadow;
pub mod upscale;

pub use cluster::{
    CLUSTER_CELL_COUNT, CLUSTER_SLICES, CLUSTER_TILES_X, CLUSTER_TILES_Y, ClusterCell, ClusterGrid,
    MAX_LIGHTS_PER_CLUSTER, assign_lights, cell_index, cell_world_corners, cell_world_sphere,
    slice_of_view_z, slice_z,
};
pub use framebuffer::{Framebuffer, Rgba8};
pub use ibl::{
    CellKey, IblProbeSet, MAX_PROBES, PROBE_CELL_SIZE, Probe, SH_A0, SH_A1, SH_A2, ShL2,
    directional_light_irradiance_closed_form,
};
pub use oracle::{ImageComparison, OracleVerdict, compare_images};
pub use post_fx::{
    TAA_ALPHA_MAX, TAA_ALPHA_MIN, TAA_DISOCCLUSION_RATIO, TAA_JITTER_PERIOD, TaaInput, TaaSample,
    bloom_composite, bloom_extract, clip_aabb, gaussian_blur_3x3, halton, jitter_for_frame,
    neighbourhood_ycgco_aabb, rgb_to_ycgco, ssao_apply, ssao_factor, taa_resolve,
    taa_resolve_pixel, tonemap_aces, ycgco_to_rgb,
};
pub use rasterize::{Vertex, Viewport, clear, rasterize_triangle};
pub use sample::{
    CubeParityScene, GoldenScene, cluster_lights_scene, combined_deferred_scene,
    golden_triangle_scene, shadow_heavy_scene,
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
pub use upscale::{
    MILESTONE_INPUT_HEIGHT, MILESTONE_INPUT_WIDTH, MILESTONE_OUTPUT_HEIGHT, MILESTONE_OUTPUT_WIDTH,
    bilinear_upscale, bilinear_upscale_to_vec,
};
