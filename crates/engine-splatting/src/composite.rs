//! `SplatCompositePass` render-graph contract (ADR-077 §4).
//!
//! Trait surface only — the GPU dispatch lives in the
//! `engine-render` integration layer alongside `splat_composite.wgsl`.
//! This module captures the *contract* (push-constant layout +
//! bind-group ABI) so `engine-render` and the importer-side tools
//! reference one canonical definition.
//!
//! The pass runs after `LightingAccumulationPass` + `IblPass` in
//! the deferred-PBR Track-A chain; it consumes the splat SoA storage
//! buffers + the sorted permutation buffer (produced by the radix
//! sort) + the GBuffer depth (for skipping behind-opaque splats);
//! it writes into the HDR scene-color target via back-to-front
//! alpha-over blending, in place.

use engine_math::Mat4;

/// Per-frame push constants the composite pass binds.
///
/// Layout matches the `SplatCompositePushConstants` struct that the
/// `splat_composite.wgsl` shader declares (ADR-077 §8). Size: 80
/// bytes (one `Mat4` = 64, one `vec2<u32>` = 8, one `vec2<u32>` = 8).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct PushConstants {
    /// World → clip transform (the splats' world positions go
    /// through this to compute screen-space billboard centres).
    pub view_projection: Mat4,
    /// Viewport extent in pixels (width, height).
    pub viewport_extent: [u32; 2],
    /// Frame counter (the composite pass does not currently use it
    /// per-frame; reserved for the TAA-history slot in v0.5).
    pub frame_idx: u64,
}

const _: () = assert!(core::mem::size_of::<PushConstants>() == 80);

/// Per-frame splat-composite pass surface.
///
/// The struct holds the resource handles the render-graph builder
/// produced. The actual `engine_render::render_graph::Pass` impl
/// lives in `engine-render`'s integration crate — putting it here
/// would force a Level-2 ↔ Level-2 dep that violates the workspace's
/// crate-layer rule. The contract definition (this module) is the
/// shared single source of truth.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SplatCompositePass {
    /// Storage-buffer handle for the splat-cloud SoA payload.
    pub splat_buffer: u32,
    /// Storage-buffer handle for the per-frame sorted permutation.
    pub sorted_perm: u32,
    /// Texture-view handle for the GBuffer depth (read-only sample).
    pub gbuffer_depth: u32,
    /// Texture-view handle for the HDR scene-color (read-write
    /// blend target).
    pub scene_color: u32,
    /// Splat count for the bound cloud (drives `vkCmdDrawIndirect`'s
    /// instance count).
    pub splat_count: u32,
}

impl SplatCompositePass {
    /// Construct with the resource handles the graph builder produced.
    pub fn new(
        splat_buffer: u32,
        sorted_perm: u32,
        gbuffer_depth: u32,
        scene_color: u32,
        splat_count: u32,
    ) -> Self {
        Self {
            splat_buffer,
            sorted_perm,
            gbuffer_depth,
            scene_color,
            splat_count,
        }
    }

    /// Stable pass name surfaced in telemetry SPAN tags.
    pub const fn name() -> &'static str {
        "splat.composite"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_constants_size_is_80_bytes() {
        assert_eq!(core::mem::size_of::<PushConstants>(), 80);
    }

    #[test]
    fn pass_name_is_stable() {
        assert_eq!(SplatCompositePass::name(), "splat.composite");
    }
}
