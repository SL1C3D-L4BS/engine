//! Reference scenes — the source-of-truth workloads the oracle
//! evaluates GPU implementations against.
//!
//! Phase 5 PR 1 ships one synthetic scene: a single red-green-blue
//! gradient triangle filling the lower half of the frame. Subsequent
//! PRs add scenes for depth pre-pass, shadow cascades, IBL probes,
//! cluster lights, TAA convergence, and upscale fidelity.
//!
//! Each scene's runner returns a [`Framebuffer`] containing the
//! committed reference image. The oracle (`oracle::compare_images`)
//! is invoked by the test harness with the GPU's output and the
//! scene's reference.

use crate::framebuffer::Framebuffer;
use crate::rasterize::{clear, rasterize_triangle, Vertex, Viewport};

/// A complete reference image + the resolution it was rendered at.
#[derive(Clone, Debug)]
pub struct GoldenScene {
    /// Stable, kebab-case name (the file-system slug for the
    /// reference image and the exception register entry).
    pub name: &'static str,
    /// The reference framebuffer.
    pub framebuffer: Framebuffer,
}

/// Render the PR-1 reference scene: an RGB-gradient triangle on a
/// black background, 128×128 sRGB.
pub fn golden_triangle_scene() -> GoldenScene {
    let mut fb = Framebuffer::new(128, 128);
    clear(&mut fb, crate::framebuffer::Rgba8::default());
    let vp = Viewport::fullframe(&fb);
    // Vertices: red bottom-left, green bottom-right, blue top.
    let tri = [
        Vertex::new(-0.9, -0.9, 0.0, 1.0, 1.0, 0.0, 0.0),
        Vertex::new(0.9, -0.9, 0.0, 1.0, 0.0, 1.0, 0.0),
        Vertex::new(0.0, 0.9, 0.0, 1.0, 0.0, 0.0, 1.0),
    ];
    rasterize_triangle(&mut fb, vp, tri);
    GoldenScene {
        name: "rgb-gradient-triangle-128",
        framebuffer: fb,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn golden_triangle_has_nonblack_centre() {
        let scene = golden_triangle_scene();
        let centre = scene.framebuffer.sample(64, 96);
        // Pixel near the bottom-centre of the triangle should have
        // a mix of red + green dominated, blue smaller.
        assert!(centre.r > 50 || centre.g > 50, "centre is black: {centre:?}");
    }
}
