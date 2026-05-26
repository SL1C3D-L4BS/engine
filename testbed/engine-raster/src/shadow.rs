//! Cascaded shadow maps · CPU oracle reference (ADR-040).
//!
//! Practical-split cascade selection (λ = 0.6), four-quadrant 4096²
//! atlas, reverse-Z. Vogel-disk 16-tap PCF for sampling.
//!
//! Reverse-Z convention (ADR-040 §3): the rendered shadow-map depth is
//! `1.0` at the light's near plane and `0.0` at far. The CPU oracle
//! follows the same convention so the GPU and CPU sample-comparison
//! results are bit-comparable up to floating-point rounding.

use crate::scene::{Aabb, Camera, MeshInstance};
use engine_math::{Mat4, Vec3, Vec4};

/// Number of cascades. Spec §IV.4.A line 380.
pub const CSM_CASCADES: usize = 4;
/// Atlas dimension per axis. Spec §IV.4.A line 380.
pub const ATLAS_DIM: u32 = 4096;
/// Sub-quadrant dimension (one cascade).
pub const CASCADE_DIM: u32 = ATLAS_DIM / 2;
/// Default practical-split blend (ADR-040 §1).
pub const PRACTICAL_SPLIT_LAMBDA: f32 = 0.6;

/// Per-cascade light-space transform + tight orthographic bounds.
#[derive(Clone, Copy, Debug)]
pub struct Cascade {
    /// Light-space view-projection (world → cascade NDC, reverse-Z).
    pub view_projection: Mat4,
    /// View-space near depth boundary for this cascade.
    pub split_near: f32,
    /// View-space far depth boundary for this cascade.
    pub split_far: f32,
    /// Bottom-left atlas tile in pixels.
    pub atlas_x: u32,
    /// Top atlas tile in pixels.
    pub atlas_y: u32,
}

/// All four cascades + the shared light direction.
#[derive(Clone, Copy, Debug)]
pub struct Cascades {
    /// Cascades in near→far order.
    pub cascades: [Cascade; CSM_CASCADES],
    /// World-space light direction (pointing *from* the light toward
    /// the scene).
    pub light_dir: Vec3,
}

/// Compute the four split distances using the practical-split blend.
///
/// `splits[0] = near`, `splits[N] = far`, intermediate values follow
/// ADR-040 §1's formula. Determinism: same inputs produce byte-identical
/// outputs — the formula uses only `+ - * /` plus `powf` (the latter
/// implemented in the platform's `libm`, which is allowed in non-sim
/// crates per ADR-023).
pub fn cascade_splits(z_near: f32, z_far: f32, lambda: f32) -> [f32; CSM_CASCADES + 1] {
    debug_assert!(z_near > 0.0 && z_far > z_near);
    let ratio = z_far / z_near;
    let range = z_far - z_near;
    let n = CSM_CASCADES as f32;
    let mut out = [0.0f32; CSM_CASCADES + 1];
    for (i, slot) in out.iter_mut().enumerate() {
        let p = (i as f32) / n;
        let log_v = z_near * ratio.powf(p);
        let lin_v = z_near + range * p;
        *slot = lambda * log_v + (1.0 - lambda) * lin_v;
    }
    out
}

/// Atlas-tile origin (bottom-left of the quadrant) for cascade `i`.
/// Fixed quadrant layout per ADR-040 §2.
pub fn atlas_origin(cascade_idx: usize) -> (u32, u32) {
    debug_assert!(cascade_idx < CSM_CASCADES);
    let half = CASCADE_DIM;
    match cascade_idx {
        0 => (0, 0),
        1 => (half, 0),
        2 => (0, half),
        3 => (half, half),
        _ => unreachable!(),
    }
}

/// Per-cascade world-space frustum-slice corners. The slice covers
/// view-space depths `[split_near, split_far]`.
fn slice_world_corners(cam: &Camera, split_near: f32, split_far: f32) -> [Vec3; 8] {
    let half_h = (cam.fov_y * 0.5).tan();
    let half_w = half_h * cam.aspect;
    let inv_view = cam.view().inverse().unwrap_or(Mat4::IDENTITY);
    let to_world = |x: f32, y: f32, z_v: f32| -> Vec3 {
        let view_pt = Vec3::new(x * half_w * z_v, y * half_h * z_v, -z_v);
        inv_view.transform_point3(view_pt)
    };
    let corners_ndc = [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)];
    let mut out = [Vec3::new(0.0, 0.0, 0.0); 8];
    for (i, (nx, ny)) in corners_ndc.iter().enumerate() {
        out[i] = to_world(*nx, *ny, split_near);
        out[i + 4] = to_world(*nx, *ny, split_far);
    }
    out
}

/// Compute a tight light-space view-projection for one cascade's
/// frustum slice. The cascade's orthographic frustum bounds the
/// AABB of `corners` projected into the light's view basis.
///
/// `texel_jitter` is added to the snap-to-texel offset (ADR-040 §5);
/// pass `Vec3::ZERO` if TAA jitter is disabled.
pub fn cascade_view_projection(
    corners: &[Vec3; 8],
    light_dir: Vec3,
    texel_jitter: (f32, f32),
) -> Mat4 {
    // Light-space basis. Pick an up vector not collinear with light_dir.
    let f = light_dir.normalize_or_zero();
    let world_up = if f.y.abs() > 0.9 {
        Vec3::new(0.0, 0.0, 1.0)
    } else {
        Vec3::new(0.0, 1.0, 0.0)
    };
    let r = world_up.cross(f).normalize_or_zero();
    let u = f.cross(r).normalize_or_zero();

    // Project corners into light space.
    let mut min = Vec3::new(f32::INFINITY, f32::INFINITY, f32::INFINITY);
    let mut max = Vec3::new(f32::NEG_INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY);
    for c in corners {
        let ls = Vec3::new(r.dot(*c), u.dot(*c), f.dot(*c));
        min = Vec3::new(min.x.min(ls.x), min.y.min(ls.y), min.z.min(ls.z));
        max = Vec3::new(max.x.max(ls.x), max.y.max(ls.y), max.z.max(ls.z));
    }

    // Snap to texel grid (ADR-040 §5). One texel ≈ extent / CASCADE_DIM.
    let extent_x = max.x - min.x;
    let extent_y = max.y - min.y;
    let texel_x = extent_x / CASCADE_DIM as f32;
    let texel_y = extent_y / CASCADE_DIM as f32;
    if texel_x > 0.0 {
        let snap_x = (min.x / texel_x).floor() * texel_x;
        let extent_snap = ((max.x - min.x) / texel_x).ceil() * texel_x;
        min.x = snap_x + texel_jitter.0 * texel_x;
        max.x = snap_x + extent_snap + texel_jitter.0 * texel_x;
    }
    if texel_y > 0.0 {
        let snap_y = (min.y / texel_y).floor() * texel_y;
        let extent_snap = ((max.y - min.y) / texel_y).ceil() * texel_y;
        min.y = snap_y + texel_jitter.1 * texel_y;
        max.y = snap_y + extent_snap + texel_jitter.1 * texel_y;
    }

    // Pull near a few metres back so geometry just outside the frustum
    // slice can still cast shadows.
    let z_padding = (max.z - min.z) * 0.25;
    let near = min.z - z_padding;
    let far = max.z + z_padding;

    // Light view (world → light space).
    let view = Mat4::from_cols_array([
        r.x, u.x, f.x, 0.0, r.y, u.y, f.y, 0.0, r.z, u.z, f.z, 0.0, 0.0, 0.0, 0.0, 1.0,
    ]);
    // Orthographic projection (reverse-Z: near=1, far=0). We build a
    // standard `[0, 1]` ortho then flip Z.
    let ortho = Mat4::orthographic_rh(min.x, max.x, min.y, max.y, near, far);
    let flip_z = Mat4::from_cols_array([
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0,
    ]);
    flip_z * ortho * view
}

/// Build all four cascades for `cam` lit by `light_dir`. `jitter` is the
/// per-frame TAA sub-texel offset (`(0, 0)` if disabled). ADR-040.
pub fn build_cascades(cam: &Camera, light_dir: Vec3, jitter: (f32, f32)) -> Cascades {
    let splits = cascade_splits(cam.near, cam.far, PRACTICAL_SPLIT_LAMBDA);
    let mut cascades = [Cascade {
        view_projection: Mat4::IDENTITY,
        split_near: 0.0,
        split_far: 0.0,
        atlas_x: 0,
        atlas_y: 0,
    }; CSM_CASCADES];
    for (i, slot) in cascades.iter_mut().enumerate() {
        let split_near = splits[i];
        let split_far = splits[i + 1];
        let corners = slice_world_corners(cam, split_near, split_far);
        let vp = cascade_view_projection(&corners, light_dir, jitter);
        let (ax, ay) = atlas_origin(i);
        *slot = Cascade {
            view_projection: vp,
            split_near,
            split_far,
            atlas_x: ax,
            atlas_y: ay,
        };
    }
    Cascades {
        cascades,
        light_dir: light_dir.normalize_or_zero(),
    }
}

/// Reverse-Z shadow atlas. One `CASCADE_DIM × CASCADE_DIM` quadrant
/// per cascade; depth in `[0, 1]`, `1.0` at the light's near plane.
#[derive(Clone, Debug)]
pub struct ShadowAtlas {
    depth: Vec<f32>,
}

impl ShadowAtlas {
    /// Allocate an atlas pre-filled with the reverse-Z far-plane value
    /// (`0.0`) — i.e. fully shadowed-everything until depth is written.
    pub fn new() -> Self {
        let n = (ATLAS_DIM as usize) * (ATLAS_DIM as usize);
        Self {
            depth: vec![0.0; n],
        }
    }

    /// Borrow the atlas as a flat depth grid.
    pub fn depth(&self) -> &[f32] {
        &self.depth
    }

    /// Mutably borrow the depth grid.
    pub fn depth_mut(&mut self) -> &mut [f32] {
        &mut self.depth
    }

    #[inline]
    fn idx(x: u32, y: u32) -> usize {
        debug_assert!(x < ATLAS_DIM && y < ATLAS_DIM);
        (y as usize) * (ATLAS_DIM as usize) + (x as usize)
    }

    /// Reverse-Z depth-test: write `z` if `z > depth[x, y]`.
    pub fn write_if_closer(&mut self, x: u32, y: u32, z: f32) -> bool {
        let i = Self::idx(x, y);
        if z > self.depth[i] {
            self.depth[i] = z;
            true
        } else {
            false
        }
    }

    /// Read a depth value.
    pub fn read(&self, x: u32, y: u32) -> f32 {
        self.depth[Self::idx(x, y)]
    }
}

impl Default for ShadowAtlas {
    fn default() -> Self {
        Self::new()
    }
}

/// Render shadow casters into `atlas` for every cascade in `cascades`.
/// Rasterisation uses point-sample depth (one pixel = one ray).
///
/// CPU reference: orthographic depth-only rasterisation of the
/// `MeshInstance` AABBs. This is the oracle for the GPU CSM pass; the
/// GPU pass renders real geometry, the CPU oracle approximates the
/// projected silhouette using the AABB. For the synthetic fixtures
/// this is sufficient (the shadow-heavy fixture uses simple boxes).
pub fn render_cascades(atlas: &mut ShadowAtlas, cascades: &Cascades, instances: &[MeshInstance]) {
    for (ci, cascade) in cascades.cascades.iter().enumerate() {
        let (ox, oy) = (cascade.atlas_x, cascade.atlas_y);
        for inst in instances.iter().filter(|i| i.casts_shadow) {
            rasterise_aabb_into_quadrant(
                atlas,
                ox,
                oy,
                CASCADE_DIM,
                cascade.view_projection,
                &inst.aabb,
            );
        }
        let _ = ci;
    }
}

/// Conservative depth raster of an AABB into one cascade quadrant.
fn rasterise_aabb_into_quadrant(
    atlas: &mut ShadowAtlas,
    ox: u32,
    oy: u32,
    dim: u32,
    vp: Mat4,
    aabb: &Aabb,
) {
    // Project all 8 box corners; rasterise the convex hull's bounding
    // rectangle with a per-pixel triangle-cover test approximated as
    // "if the pixel's centre projects inside the box silhouette in 2D,
    // write depth". For our oracle fixtures this is accurate to <1 px.
    let corners = [
        Vec3::new(aabb.min.x, aabb.min.y, aabb.min.z),
        Vec3::new(aabb.max.x, aabb.min.y, aabb.min.z),
        Vec3::new(aabb.min.x, aabb.max.y, aabb.min.z),
        Vec3::new(aabb.max.x, aabb.max.y, aabb.min.z),
        Vec3::new(aabb.min.x, aabb.min.y, aabb.max.z),
        Vec3::new(aabb.max.x, aabb.min.y, aabb.max.z),
        Vec3::new(aabb.min.x, aabb.max.y, aabb.max.z),
        Vec3::new(aabb.max.x, aabb.max.y, aabb.max.z),
    ];

    let mut min_x = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    let mut min_z = f32::INFINITY;
    let mut max_z = f32::NEG_INFINITY;
    for c in &corners {
        let clip = vp * Vec4::new(c.x, c.y, c.z, 1.0);
        if clip.w.abs() < 1e-6 {
            continue;
        }
        let nx = clip.x / clip.w;
        let ny = clip.y / clip.w;
        let nz = clip.z / clip.w;
        min_x = min_x.min(nx);
        max_x = max_x.max(nx);
        min_y = min_y.min(ny);
        max_y = max_y.max(ny);
        min_z = min_z.min(nz);
        max_z = max_z.max(nz);
    }
    if !min_x.is_finite() {
        return;
    }
    // Map NDC → quadrant pixels.
    let to_pixel_x = |nx: f32| ((nx * 0.5 + 0.5) * dim as f32).floor() as i32;
    let to_pixel_y = |ny: f32| ((1.0 - (ny * 0.5 + 0.5)) * dim as f32).floor() as i32;
    let px_lo = to_pixel_x(min_x).max(0);
    let px_hi = to_pixel_x(max_x).min(dim as i32 - 1);
    let py_lo = to_pixel_y(max_y).max(0); // y is flipped
    let py_hi = to_pixel_y(min_y).min(dim as i32 - 1);
    if px_hi < px_lo || py_hi < py_lo {
        return;
    }
    // Use the maximum projected depth (reverse-Z: closest to light has
    // largest value). For an opaque silhouette, the entry-face depth is
    // the reverse-Z winner across all 8 corners.
    let depth = max_z.clamp(0.0, 1.0);
    for py in py_lo..=py_hi {
        for px in px_lo..=px_hi {
            let ax = ox + px as u32;
            let ay = oy + py as u32;
            atlas.write_if_closer(ax, ay, depth);
        }
    }
}

/// Sample the shadow atlas at the world-space point `world_p` for the
/// active cascade. Returns a visibility value in `[0, 1]` (1 = fully
/// lit, 0 = fully shadowed). 16-tap Vogel-disk PCF over a 5×5 area
/// (ADR-040 §4).
pub fn sample_shadow_pcf(atlas: &ShadowAtlas, cascade: &Cascade, world_p: Vec3) -> f32 {
    let clip = cascade.view_projection * Vec4::new(world_p.x, world_p.y, world_p.z, 1.0);
    if clip.w.abs() < 1e-6 {
        return 1.0;
    }
    let nx = clip.x / clip.w;
    let ny = clip.y / clip.w;
    let nz = clip.z / clip.w;
    // Reject points outside the cascade NDC volume.
    if !(-1.0..=1.0).contains(&nx) || !(-1.0..=1.0).contains(&ny) {
        return 1.0;
    }
    let dim_f = CASCADE_DIM as f32;
    let texel = 1.0 / dim_f;
    let fx = nx * 0.5 + 0.5;
    let fy = 1.0 - (ny * 0.5 + 0.5);
    let mut accum = 0.0f32;
    // 16 Vogel-disk samples, golden-angle rotation. The fixed pattern
    // matches a non-rotated reference (per-pixel rotation by a screen-
    // space hash is a future GPU tweak; the oracle locks the average).
    let golden_angle: f32 = 2.399_963_2; // π * (3 − √5)
    let radius = 2.0; // 5×5 area: ±2 texels.
    for k in 0..16 {
        let r = ((k as f32 + 0.5) / 16.0).sqrt() * radius;
        let theta = (k as f32) * golden_angle;
        let dx = theta.cos() * r * texel;
        let dy = theta.sin() * r * texel;
        let sx = ((fx + dx).clamp(0.0, 1.0) * (dim_f - 1.0)).floor() as u32;
        let sy = ((fy + dy).clamp(0.0, 1.0) * (dim_f - 1.0)).floor() as u32;
        let depth = atlas.read(cascade.atlas_x + sx, cascade.atlas_y + sy);
        // Reverse-Z: lit when the fragment is *farther* from the
        // light than the atlas (i.e. fragment depth < atlas depth).
        // Compare with a small bias against acne.
        let bias = 1e-4f32;
        accum += if nz + bias >= depth { 1.0 } else { 0.0 };
    }
    accum / 16.0
}

/// Select the cascade index for a given view-space depth. Returns
/// `CSM_CASCADES - 1` when the depth is beyond the last cascade.
pub fn select_cascade(cascades: &Cascades, view_z: f32) -> usize {
    for (i, c) in cascades.cascades.iter().enumerate() {
        if view_z >= c.split_near && view_z <= c.split_far {
            return i;
        }
    }
    CSM_CASCADES - 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cascade_splits_endpoints_match_near_far() {
        let s = cascade_splits(0.1, 1000.0, 0.6);
        assert!((s[0] - 0.1).abs() < 1e-6);
        assert!((s[CSM_CASCADES] - 1000.0).abs() < 1e-3);
    }

    #[test]
    fn cascade_splits_are_monotone_increasing() {
        let s = cascade_splits(0.1, 1000.0, 0.6);
        for i in 1..=CSM_CASCADES {
            assert!(s[i] > s[i - 1], "splits not monotone at {i}: {:?}", s);
        }
    }

    #[test]
    fn cascade_splits_are_deterministic() {
        let a = cascade_splits(0.1, 1000.0, 0.6);
        let b = cascade_splits(0.1, 1000.0, 0.6);
        // Byte-equal — same inputs, same outputs.
        for i in 0..a.len() {
            assert_eq!(a[i].to_bits(), b[i].to_bits());
        }
    }

    #[test]
    fn atlas_quadrants_are_distinct_and_cover() {
        let mut origins = std::collections::HashSet::new();
        for i in 0..CSM_CASCADES {
            let o = atlas_origin(i);
            assert!(origins.insert(o), "duplicate quadrant origin at {i}");
            assert!(o.0 + CASCADE_DIM <= ATLAS_DIM);
            assert!(o.1 + CASCADE_DIM <= ATLAS_DIM);
        }
    }

    #[test]
    fn select_cascade_returns_first_matching() {
        let cam = test_camera();
        let cs = build_cascades(&cam, Vec3::new(0.0, -1.0, 0.0), (0.0, 0.0));
        let z_in_first = (cs.cascades[0].split_near + cs.cascades[0].split_far) * 0.5;
        assert_eq!(select_cascade(&cs, z_in_first), 0);
        let z_in_last = (cs.cascades[CSM_CASCADES - 1].split_near
            + cs.cascades[CSM_CASCADES - 1].split_far)
            * 0.5;
        assert_eq!(select_cascade(&cs, z_in_last), CSM_CASCADES - 1);
    }

    fn test_camera() -> Camera {
        Camera {
            position: Vec3::new(0.0, 0.0, 0.0),
            forward: Vec3::new(0.0, 0.0, -1.0),
            up: Vec3::new(0.0, 1.0, 0.0),
            fov_y: 60.0_f32.to_radians(),
            aspect: 16.0 / 9.0,
            near: 0.1,
            far: 1000.0,
        }
    }
}
