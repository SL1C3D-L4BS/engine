//! Clustered-light binning · CPU oracle reference (ADR-043).
//!
//! The cluster grid is `CLUSTER_TILES_X × CLUSTER_TILES_Y × CLUSTER_SLICES`
//! cells. Each cell stores up to [`MAX_LIGHTS_PER_CLUSTER`] 16-bit light
//! indices. Logarithmic depth slicing per ADR-043 §1:
//!
//! ```text
//! slice_z(i) = z_near · (z_far / z_near)^(i / CLUSTER_SLICES)
//! ```
//!
//! The GPU compute shader (`light.cluster` pass) and this reference
//! must produce *set-identical* per-cell light lists (order may
//! differ). `assign_lights` is the per-frame entry point.

use crate::scene::{Camera, Light, LightType};
use engine_math::{Mat4, Vec3};

/// Tile count along the x-axis (spatial). Spec §IV.4.A line 381.
pub const CLUSTER_TILES_X: u32 = 16;
/// Tile count along the y-axis (spatial). Spec §IV.4.A line 381.
pub const CLUSTER_TILES_Y: u32 = 9;
/// Depth-slice count. Spec §IV.4.A line 381.
pub const CLUSTER_SLICES: u32 = 24;
/// Per-cell light cap per ADR-043 §2.
pub const MAX_LIGHTS_PER_CLUSTER: usize = 32;
/// Total cell count.
pub const CLUSTER_CELL_COUNT: u32 = CLUSTER_TILES_X * CLUSTER_TILES_Y * CLUSTER_SLICES;

/// Depth distance to slice boundary `i ∈ 0..=CLUSTER_SLICES`. Returns
/// the view-space `z_view` (positive, in metres) at the boundary.
///
/// Slice `i` covers `[slice_z(i), slice_z(i+1)]`. ADR-043 §1.
pub fn slice_z(i: u32, z_near: f32, z_far: f32) -> f32 {
    debug_assert!(i <= CLUSTER_SLICES);
    debug_assert!(z_near > 0.0 && z_far > z_near);
    let t = (i as f32) / (CLUSTER_SLICES as f32);
    z_near * (z_far / z_near).powf(t)
}

/// Inverse of [`slice_z`]: which slice does this view-space depth fall
/// into? Clamps to `[0, CLUSTER_SLICES - 1]` so out-of-range fragments
/// land in the boundary cluster.
pub fn slice_of_view_z(z_view: f32, z_near: f32, z_far: f32) -> u32 {
    debug_assert!(z_near > 0.0 && z_far > z_near);
    let z = z_view.max(z_near);
    let log_ratio = (z / z_near).ln() / (z_far / z_near).ln();
    let i = (log_ratio * CLUSTER_SLICES as f32).floor() as i32;
    i.clamp(0, CLUSTER_SLICES as i32 - 1) as u32
}

/// Flatten a `(tile_x, tile_y, slice)` tuple to the cell index used by
/// the cluster SSBO. ADR-043 §5 fragment-shader formula.
#[inline]
pub fn cell_index(tile_x: u32, tile_y: u32, slice: u32) -> u32 {
    debug_assert!(tile_x < CLUSTER_TILES_X);
    debug_assert!(tile_y < CLUSTER_TILES_Y);
    debug_assert!(slice < CLUSTER_SLICES);
    slice * (CLUSTER_TILES_X * CLUSTER_TILES_Y) + tile_y * CLUSTER_TILES_X + tile_x
}

/// One cluster cell — light count + bounded index list. Mirrors the
/// `ClusterCell` shape from ADR-043 §2 (the GPU layout adds explicit
/// padding; here we keep the logical record).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClusterCell {
    /// Number of valid entries in `light_indices`.
    pub light_count: u8,
    /// Light indices (only `light_indices[..light_count]` is valid).
    pub light_indices: [u16; MAX_LIGHTS_PER_CLUSTER],
}

impl Default for ClusterCell {
    fn default() -> Self {
        Self {
            light_count: 0,
            light_indices: [0; MAX_LIGHTS_PER_CLUSTER],
        }
    }
}

impl ClusterCell {
    /// Append `light_idx` if there is room. Returns `true` on success,
    /// `false` if the cell was already at the 32-light cap (the
    /// overflow telemetry counter is incremented by [`assign_lights`]
    /// when this happens).
    pub fn push(&mut self, light_idx: u16) -> bool {
        if (self.light_count as usize) < MAX_LIGHTS_PER_CLUSTER {
            self.light_indices[self.light_count as usize] = light_idx;
            self.light_count += 1;
            true
        } else {
            false
        }
    }

    /// Iterate the valid light indices in registration order.
    pub fn lights(&self) -> impl Iterator<Item = u16> + '_ {
        self.light_indices[..self.light_count as usize]
            .iter()
            .copied()
    }
}

/// Full per-frame cluster grid. The CPU oracle owns one; the GPU
/// `light.cluster` pass writes the same data into an SSBO.
#[derive(Clone, Debug)]
pub struct ClusterGrid {
    cells: Vec<ClusterCell>,
    /// Count of "tried to insert into a full cell" events. Maps to
    /// `COUNTER "render.cluster_light_overflow"` (ADR-043 §4).
    pub overflow_count: u32,
}

impl ClusterGrid {
    /// Allocate an empty grid.
    pub fn new() -> Self {
        Self {
            cells: vec![ClusterCell::default(); CLUSTER_CELL_COUNT as usize],
            overflow_count: 0,
        }
    }

    /// Borrow the per-cell view.
    pub fn cells(&self) -> &[ClusterCell] {
        &self.cells
    }

    /// Reset every cell to empty and clear the overflow counter.
    pub fn reset(&mut self) {
        for c in &mut self.cells {
            *c = ClusterCell::default();
        }
        self.overflow_count = 0;
    }

    /// Look up the cell at `(tile_x, tile_y, slice)`.
    pub fn cell(&self, tile_x: u32, tile_y: u32, slice: u32) -> &ClusterCell {
        &self.cells[cell_index(tile_x, tile_y, slice) as usize]
    }

    fn cell_mut(&mut self, tile_x: u32, tile_y: u32, slice: u32) -> &mut ClusterCell {
        &mut self.cells[cell_index(tile_x, tile_y, slice) as usize]
    }
}

impl Default for ClusterGrid {
    fn default() -> Self {
        Self::new()
    }
}

/// Build the world-space frustum of one cluster cell. The cell spans
/// `tile_x .. tile_x+1` along x, `tile_y .. tile_y+1` along y in screen
/// space, and `[slice_z(slice), slice_z(slice+1)]` in view-space depth.
///
/// Returns 8 corner positions in world space (the slab of 4 near + 4
/// far corners). The corners are ordered (near-bl, near-br, near-tl,
/// near-tr, far-bl, far-br, far-tl, far-tr).
pub fn cell_world_corners(cam: &Camera, tile_x: u32, tile_y: u32, slice: u32) -> [Vec3; 8] {
    let z_near = slice_z(slice, cam.near, cam.far);
    let z_far = slice_z(slice + 1, cam.near, cam.far);

    let tile_w = 1.0 / CLUSTER_TILES_X as f32;
    let tile_h = 1.0 / CLUSTER_TILES_Y as f32;
    let u0 = tile_x as f32 * tile_w;
    let u1 = u0 + tile_w;
    let v0 = tile_y as f32 * tile_h;
    let v1 = v0 + tile_h;

    // NDC corners of the tile (x in [-1, 1], y in [-1, 1]).
    let ndc_x = |u: f32| u * 2.0 - 1.0;
    let ndc_y = |v: f32| v * 2.0 - 1.0;
    let ndc_corners = [
        (ndc_x(u0), ndc_y(v0)),
        (ndc_x(u1), ndc_y(v0)),
        (ndc_x(u0), ndc_y(v1)),
        (ndc_x(u1), ndc_y(v1)),
    ];

    // View-space basis: the camera looks down -Z view-axis (right-handed).
    // Camera-space corner = (ndc_x * aspect_w * |z|, ndc_y * h * |z|, -z).
    let half_h_at_z = (cam.fov_y * 0.5).tan();
    let half_w_at_z = half_h_at_z * cam.aspect;

    let inv_view = cam.view().inverse().unwrap_or(Mat4::IDENTITY);
    let to_world = |x: f32, y: f32, z_v: f32| -> Vec3 {
        // View-space → world.
        let view_pt = Vec3::new(x * half_w_at_z * z_v, y * half_h_at_z * z_v, -z_v);
        inv_view.transform_point3(view_pt)
    };

    let mut out = [Vec3::new(0.0, 0.0, 0.0); 8];
    for (i, (nx, ny)) in ndc_corners.iter().enumerate() {
        out[i] = to_world(*nx, *ny, z_near);
        out[i + 4] = to_world(*nx, *ny, z_far);
    }
    out
}

/// Bounding sphere (centre, radius) of one cluster cell in world space.
/// The CPU sphere-vs-sphere test (`assign_lights`) uses this to bin
/// point lights.
pub fn cell_world_sphere(cam: &Camera, tile_x: u32, tile_y: u32, slice: u32) -> (Vec3, f32) {
    let corners = cell_world_corners(cam, tile_x, tile_y, slice);
    let mut centre = Vec3::new(0.0, 0.0, 0.0);
    for c in &corners {
        centre = Vec3::new(centre.x + c.x, centre.y + c.y, centre.z + c.z);
    }
    let inv8 = 1.0 / 8.0;
    centre = Vec3::new(centre.x * inv8, centre.y * inv8, centre.z * inv8);
    let mut r2 = 0.0f32;
    for c in &corners {
        let d = Vec3::new(c.x - centre.x, c.y - centre.y, c.z - centre.z);
        r2 = r2.max(d.length_squared());
    }
    (centre, r2.sqrt())
}

/// Assign every light in `lights` to the cluster cells it overlaps.
/// Returns the populated grid.
///
/// For `LightType::Point` the test is sphere–sphere (light sphere
/// centred at the light, vs each cell's enclosing world-space sphere).
/// For `LightType::Directional` every cell is tagged — a directional
/// light affects all fragments.
pub fn assign_lights(cam: &Camera, lights: &[Light]) -> ClusterGrid {
    let mut grid = ClusterGrid::new();
    if lights.is_empty() {
        return grid;
    }

    // Pre-compute per-cell spheres once. 3 456 cells × ~24 bytes ≈ 80 KiB,
    // acceptable for the CPU reference and matches the GPU-side
    // workgroup pattern (ADR-043 §4) where each workgroup computes its
    // own tile column's spheres.
    let mut cell_spheres: Vec<(Vec3, f32)> = Vec::with_capacity(CLUSTER_CELL_COUNT as usize);
    for slice in 0..CLUSTER_SLICES {
        for ty in 0..CLUSTER_TILES_Y {
            for tx in 0..CLUSTER_TILES_X {
                cell_spheres.push(cell_world_sphere(cam, tx, ty, slice));
            }
        }
    }

    for (idx, light) in lights.iter().enumerate() {
        let light_idx = idx as u16;
        match light.kind {
            LightType::Directional => {
                for slice in 0..CLUSTER_SLICES {
                    for ty in 0..CLUSTER_TILES_Y {
                        for tx in 0..CLUSTER_TILES_X {
                            let cell = grid.cell_mut(tx, ty, slice);
                            if !cell.push(light_idx) {
                                grid.overflow_count += 1;
                            }
                        }
                    }
                }
            }
            LightType::Point => {
                // Light sphere centre = position; radius = range.
                let centre = light.position_or_direction;
                let radius = light.range;
                for (cell_i, (c_centre, c_radius)) in cell_spheres.iter().enumerate() {
                    let dx = c_centre.x - centre.x;
                    let dy = c_centre.y - centre.y;
                    let dz = c_centre.z - centre.z;
                    let dist2 = dx * dx + dy * dy + dz * dz;
                    let sum_r = c_radius + radius;
                    if dist2 <= sum_r * sum_r {
                        let cell = &mut grid.cells[cell_i];
                        if !cell.push(light_idx) {
                            grid.overflow_count += 1;
                        }
                    }
                }
            }
        }
    }

    grid
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slice_z_matches_closed_form_endpoints() {
        let near = 0.1f32;
        let far = 1000.0f32;
        assert!((slice_z(0, near, far) - near).abs() < 1e-3);
        assert!((slice_z(CLUSTER_SLICES, near, far) - far).abs() < 1e-1);
        // Geometric mean property: log-midpoint slice equals geometric mean.
        let mid = slice_z(CLUSTER_SLICES / 2, near, far);
        let expected = (near * far).sqrt();
        assert!(
            (mid / expected - 1.0).abs() < 1e-3,
            "log-midpoint mismatch: got {mid}, expected {expected}"
        );
    }

    #[test]
    fn slice_of_view_z_inverts_slice_z_at_midpoints() {
        let near = 0.1f32;
        let far = 1000.0f32;
        for i in 0..CLUSTER_SLICES {
            let z_lo = slice_z(i, near, far);
            let z_hi = slice_z(i + 1, near, far);
            // Midpoint depth must fall in slice i (geometric mean).
            let mid = (z_lo * z_hi).sqrt();
            assert_eq!(slice_of_view_z(mid, near, far), i, "slice {i} round-trip");
        }
    }

    #[test]
    fn cell_index_is_dense_unique_total() {
        let mut seen = vec![false; CLUSTER_CELL_COUNT as usize];
        for slice in 0..CLUSTER_SLICES {
            for ty in 0..CLUSTER_TILES_Y {
                for tx in 0..CLUSTER_TILES_X {
                    let idx = cell_index(tx, ty, slice);
                    assert!(!seen[idx as usize]);
                    seen[idx as usize] = true;
                }
            }
        }
        assert!(seen.into_iter().all(|x| x));
    }

    #[test]
    fn cluster_cell_push_caps_at_max() {
        let mut c = ClusterCell::default();
        for i in 0..MAX_LIGHTS_PER_CLUSTER as u16 {
            assert!(c.push(i));
        }
        assert_eq!(c.light_count as usize, MAX_LIGHTS_PER_CLUSTER);
        assert!(!c.push(0xFFFF), "32-light cap must be enforced");
    }

    #[test]
    fn directional_light_assigns_to_every_cell() {
        let cam = test_camera();
        let lights = [Light::directional(
            Vec3::new(0.0, -1.0, 0.0),
            Vec3::new(1.0, 1.0, 1.0),
            1.0,
        )];
        let g = assign_lights(&cam, &lights);
        for c in g.cells() {
            assert_eq!(c.light_count, 1);
            assert_eq!(c.light_indices[0], 0);
        }
        assert_eq!(g.overflow_count, 0);
    }

    #[test]
    fn point_light_far_from_camera_is_not_assigned_near_clusters() {
        let cam = test_camera();
        // Light 500 m behind the camera (positive z view-space, i.e. behind).
        let lights = [Light::point(
            Vec3::new(0.0, 0.0, 500.0),
            Vec3::new(1.0, 0.0, 0.0),
            1.0,
            0.5, // tiny radius
        )];
        let g = assign_lights(&cam, &lights);
        // The first-slice front-tile cell must be empty (the light is
        // out of range).
        let near_cell = g.cell(0, 0, 0);
        assert_eq!(
            near_cell.light_count, 0,
            "far light should not light a near cluster"
        );
    }

    #[test]
    fn overflow_counter_increments_when_capped() {
        let cam = test_camera();
        // 33 directional lights — each one tries to populate every cell,
        // so the 33rd push fails everywhere → overflow_count = total cells.
        let lights: Vec<Light> = (0..33)
            .map(|_| Light::directional(Vec3::new(0.0, -1.0, 0.0), Vec3::new(1.0, 1.0, 1.0), 1.0))
            .collect();
        let g = assign_lights(&cam, &lights);
        assert_eq!(g.overflow_count, CLUSTER_CELL_COUNT);
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
