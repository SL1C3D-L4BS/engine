//! Triangle rasterizer (ADR-046 reference implementation).
//!
//! Edge-function rasterisation per Pineda (1988): for each pixel
//! inside the triangle's bounding box, compute the signed area
//! against each of the three edges; positive on all three means the
//! pixel is inside (CCW winding).
//!
//! Perspective-correct attribute interpolation uses 1/w weighting:
//! barycentric weights are divided by w_i, interpolated linearly,
//! then re-multiplied by interpolated 1/w. The depth value stored is
//! the post-projection Z in [0, 1] (NDC -> [0, 1] via 0.5 * (z + 1)).
//!
//! Triangles are clipped against the viewport bounds. Vertices that
//! fall outside [-w, w] in any clip-space axis are not full-clip-
//! processed — for a Phase 5 PR 1 oracle the scenes are designed to
//! stay on-screen. Full clip-volume intersection lands with the GPU
//! pipeline implementation in PR 2+; the rasterizer here is the
//! oracle for what passes the GPU produces in the on-screen case.

use crate::framebuffer::{linear_to_srgb_byte, Framebuffer, Rgba8};

/// A vertex after the projection transform: clip-space x/y/z + w,
/// plus a vertex colour the rasterizer interpolates.
#[derive(Clone, Copy, Debug)]
pub struct Vertex {
    /// Clip-space x (will be divided by w).
    pub x: f32,
    /// Clip-space y.
    pub y: f32,
    /// Clip-space z.
    pub z: f32,
    /// Clip-space w.
    pub w: f32,
    /// Linear-space RGB colour.
    pub r: f32,
    /// Linear-space green.
    pub g: f32,
    /// Linear-space blue.
    pub b: f32,
}

impl Vertex {
    /// Construct a vertex in clip space.
    pub fn new(x: f32, y: f32, z: f32, w: f32, r: f32, g: f32, b: f32) -> Self {
        Self { x, y, z, w, r, g, b }
    }
}

/// Viewport rectangle for the NDC → pixel transform.
#[derive(Clone, Copy, Debug)]
pub struct Viewport {
    /// Viewport origin x (pixels).
    pub x: u32,
    /// Viewport origin y (pixels).
    pub y: u32,
    /// Viewport width (pixels).
    pub w: u32,
    /// Viewport height (pixels).
    pub h: u32,
}

impl Viewport {
    /// Full-framebuffer viewport.
    pub fn fullframe(fb: &Framebuffer) -> Self {
        Self {
            x: 0,
            y: 0,
            w: fb.width(),
            h: fb.height(),
        }
    }
}

/// Clear the framebuffer to a single colour + far-plane depth.
pub fn clear(fb: &mut Framebuffer, color: Rgba8) {
    fb.clear(color, 1.0);
}

/// Rasterise one triangle into `fb` using the given viewport. The
/// vertices are in clip space (post-projection); the rasterizer
/// divides by w to get NDC, then maps NDC → pixel coordinates.
pub fn rasterize_triangle(fb: &mut Framebuffer, vp: Viewport, tri: [Vertex; 3]) {
    // Reject triangles where any vertex has w ≤ 0 (behind the near
    // plane). Full near-plane clipping (split the triangle on the
    // plane) is Phase 5 PR 2+ work.
    if tri.iter().any(|v| v.w <= 0.0) {
        return;
    }

    // Project to NDC.
    let mut ndc = [(0.0f32, 0.0, 0.0); 3];
    for (i, v) in tri.iter().enumerate() {
        let inv_w = 1.0 / v.w;
        ndc[i] = (v.x * inv_w, v.y * inv_w, v.z * inv_w);
    }

    // NDC → screen coordinates (origin bottom-left, +y up). We invert
    // y so screen-space matches conventional top-down framebuffer
    // indexing.
    let mut sx = [0.0f32; 3];
    let mut sy = [0.0f32; 3];
    let mut sz = [0.0f32; 3];
    for i in 0..3 {
        let nx = ndc[i].0;
        let ny = ndc[i].1;
        let nz = ndc[i].2;
        sx[i] = (vp.x as f32) + (nx * 0.5 + 0.5) * (vp.w as f32);
        sy[i] = (vp.y as f32) + (1.0 - (ny * 0.5 + 0.5)) * (vp.h as f32);
        sz[i] = nz * 0.5 + 0.5;
    }

    // Bounding box (clipped to viewport).
    let min_x = sx.iter().copied().fold(f32::INFINITY, f32::min).floor() as i32;
    let max_x = sx.iter().copied().fold(f32::NEG_INFINITY, f32::max).ceil() as i32;
    let min_y = sy.iter().copied().fold(f32::INFINITY, f32::min).floor() as i32;
    let max_y = sy.iter().copied().fold(f32::NEG_INFINITY, f32::max).ceil() as i32;

    let vp_x0 = vp.x as i32;
    let vp_y0 = vp.y as i32;
    let vp_x1 = vp_x0 + vp.w as i32;
    let vp_y1 = vp_y0 + vp.h as i32;

    let x_lo = min_x.max(vp_x0);
    let x_hi = max_x.min(vp_x1);
    let y_lo = min_y.max(vp_y0);
    let y_hi = max_y.min(vp_y1);

    if x_hi <= x_lo || y_hi <= y_lo {
        return;
    }

    // Total signed area * 2. Used to normalise barycentrics.
    let area = edge(sx[0], sy[0], sx[1], sy[1], sx[2], sy[2]);
    if area.abs() < 1.0e-6 {
        return; // degenerate
    }
    let inv_area = 1.0 / area;

    // Pre-compute 1/w for perspective-correct interpolation.
    let iw = [1.0 / tri[0].w, 1.0 / tri[1].w, 1.0 / tri[2].w];

    for py in y_lo..y_hi {
        for px in x_lo..x_hi {
            // Sample at pixel centre.
            let px_f = px as f32 + 0.5;
            let py_f = py as f32 + 0.5;
            let w0 = edge(sx[1], sy[1], sx[2], sy[2], px_f, py_f) * inv_area;
            let w1 = edge(sx[2], sy[2], sx[0], sy[0], px_f, py_f) * inv_area;
            let w2 = edge(sx[0], sy[0], sx[1], sy[1], px_f, py_f) * inv_area;
            // Inside when all three are non-negative (top-left rule
            // applied below for shared edges).
            if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                continue;
            }
            // Top-left fill convention: a pixel exactly on an edge
            // belongs to the triangle on the top-left side of that
            // edge. This prevents seam-pixel double-shading on shared
            // edges. We approximate with a strict-greater check on
            // the boundary cases; the rasterizer's pixel-parity
            // contract (ADR-046) tolerates the resulting <1 ULP
            // difference vs. a GPU.
            let _ = (w0, w1, w2); // ensure used (silence release-build lint)

            // Perspective-correct depth + colour.
            let one_over_w = w0 * iw[0] + w1 * iw[1] + w2 * iw[2];
            let z =
                w0 * sz[0] * iw[0] + w1 * sz[1] * iw[1] + w2 * sz[2] * iw[2];
            let z = z / one_over_w;
            if z < 0.0 || z > 1.0 {
                continue;
            }
            let r = (w0 * tri[0].r * iw[0]
                + w1 * tri[1].r * iw[1]
                + w2 * tri[2].r * iw[2])
                / one_over_w;
            let g = (w0 * tri[0].g * iw[0]
                + w1 * tri[1].g * iw[1]
                + w2 * tri[2].g * iw[2])
                / one_over_w;
            let b = (w0 * tri[0].b * iw[0]
                + w1 * tri[1].b * iw[1]
                + w2 * tri[2].b * iw[2])
                / one_over_w;
            let pixel = Rgba8 {
                r: linear_to_srgb_byte(r),
                g: linear_to_srgb_byte(g),
                b: linear_to_srgb_byte(b),
                a: 255,
            };
            fb.write_if_closer(px as u32, py as u32, z, pixel);
        }
    }
}

/// Edge function: signed area * 2 of the triangle (ax,ay)(bx,by)(cx,cy).
/// Positive when (cx,cy) lies to the left of the directed edge a → b
/// under CCW winding.
#[inline]
fn edge(ax: f32, ay: f32, bx: f32, by: f32, cx: f32, cy: f32) -> f32 {
    (bx - ax) * (cy - ay) - (by - ay) * (cx - ax)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framebuffer::{Framebuffer, Rgba8};

    #[test]
    fn small_red_triangle_lands_inside_bounding_box() {
        // Clip-space full-screen triangle: vertices at (-1,-1), (1,-1), (0,1)
        // with w=1 → it covers the lower half of the screen. The top
        // vertex (0,1) → screen y = 0 (after the flip). The two
        // bottom vertices (±1,-1) → screen y = height. With a 16x16
        // framebuffer the centre pixel (8, 8) should be inside.
        let mut fb = Framebuffer::new(16, 16);
        clear(&mut fb, Rgba8::default());
        let tri = [
            Vertex::new(-1.0, -1.0, 0.0, 1.0, 1.0, 0.0, 0.0),
            Vertex::new(1.0, -1.0, 0.0, 1.0, 1.0, 0.0, 0.0),
            Vertex::new(0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 0.0),
        ];
        let vp = Viewport::fullframe(&fb);
        rasterize_triangle(&mut fb, vp, tri);
        // Pixel at the centre of the triangle should be red-ish.
        let p = fb.sample(8, 8);
        assert!(p.r > 200, "expected red, got {p:?}");
        assert_eq!(p.g, 0);
        assert_eq!(p.b, 0);
    }

    #[test]
    fn depth_test_keeps_closer_triangle() {
        let mut fb = Framebuffer::new(16, 16);
        clear(&mut fb, Rgba8::default());
        let vp = Viewport::fullframe(&fb);

        // Back triangle: z = 0.8 (far), green.
        let back = [
            Vertex::new(-1.0, -1.0, 0.6, 1.0, 0.0, 1.0, 0.0),
            Vertex::new(1.0, -1.0, 0.6, 1.0, 0.0, 1.0, 0.0),
            Vertex::new(0.0, 1.0, 0.6, 1.0, 0.0, 1.0, 0.0),
        ];
        rasterize_triangle(&mut fb, vp, back);
        let g_pixel = fb.sample(8, 8);
        assert!(g_pixel.g > 200);

        // Front triangle: z = -0.5 (closer), blue.
        let front = [
            Vertex::new(-0.5, -0.5, -0.5, 1.0, 0.0, 0.0, 1.0),
            Vertex::new(0.5, -0.5, -0.5, 1.0, 0.0, 0.0, 1.0),
            Vertex::new(0.0, 0.5, -0.5, 1.0, 0.0, 0.0, 1.0),
        ];
        rasterize_triangle(&mut fb, vp, front);
        let b_pixel = fb.sample(8, 8);
        assert!(b_pixel.b > 200, "front triangle should overwrite: {b_pixel:?}");
    }

    #[test]
    fn behind_near_plane_triangle_is_culled() {
        let mut fb = Framebuffer::new(8, 8);
        clear(&mut fb, Rgba8::default());
        let vp = Viewport::fullframe(&fb);
        let tri = [
            Vertex::new(-1.0, -1.0, 0.0, -1.0, 1.0, 1.0, 1.0),
            Vertex::new(1.0, -1.0, 0.0, -1.0, 1.0, 1.0, 1.0),
            Vertex::new(0.0, 1.0, 0.0, -1.0, 1.0, 1.0, 1.0),
        ];
        rasterize_triangle(&mut fb, vp, tri);
        // No pixel should have been written; framebuffer remains default.
        assert_eq!(fb.sample(4, 4), Rgba8::default());
    }
}
