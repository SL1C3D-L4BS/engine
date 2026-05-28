//! Reference scenes — the source-of-truth workloads the oracle
//! evaluates GPU implementations against.
//!
//! Phase 5 PR 1 ships one synthetic scene: a single red-green-blue
//! gradient triangle filling the lower half of the frame. Subsequent
//! PRs add scenes for depth pre-pass, shadow cascades, IBL probes,
//! cluster lights, TAA convergence, and upscale fidelity.
//!
//! Phase 5 PR 3 adds three deferred fixtures (ADR-053 §PR3):
//! - [`cluster_lights_scene`] — clusters-only smoke. Synthetic
//!   forward-camera scene with a handful of point lights; the
//!   `cluster_assignment_oracle` test compares CPU vs the would-be
//!   GPU output.
//! - [`shadow_heavy_scene`] — CSM-heavy scene with one directional
//!   light + many casters; the `csm_atlas_pixel_parity` test reads
//!   the rendered atlas.
//! - [`combined_deferred_scene`] — clusters + CSM + Cook-Torrance
//!   together; the `cluster_pixel_parity` test exercises the full
//!   accumulation path.
//!
//! Each scene's runner returns a [`Framebuffer`] containing the
//! committed reference image. The oracle (`oracle::compare_images`)
//! is invoked by the test harness with the GPU's output and the
//! scene's reference.

use crate::cluster::{ClusterGrid, assign_lights};
use crate::framebuffer::{Framebuffer, Rgba8, linear_to_srgb_byte};
use crate::post_fx::tonemap_aces;
use crate::rasterize::{Vertex, Viewport, clear, rasterize_triangle};
use crate::scene::{Aabb, Camera, Light, Material, MeshInstance};
use crate::shading::{SurfaceFragment, accumulate_lighting, cook_torrance};
use crate::shadow::{Cascades, ShadowAtlas, build_cascades, render_cascades};
use engine_math::{Mat4, Vec3, Vec4};

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

/// Fixture: a 5×5 grid of point lights laid out on a horizontal plane,
/// looking down. The cluster oracle (`cluster_assignment_oracle`) reads
/// this layout and compares the CPU reference's per-cell light lists
/// against the GPU. The image is a *colour-coded* visualisation: each
/// pixel's RGB encodes the number of lights overlapping that pixel's
/// cluster cell (so the test harness can also do an image-level diff).
pub fn cluster_lights_scene() -> (GoldenScene, Camera, Vec<Light>, ClusterGrid) {
    let cam = Camera {
        position: Vec3::new(0.0, 5.0, 0.0),
        forward: Vec3::new(0.0, -0.5, -1.0).normalize_or_zero(),
        up: Vec3::new(0.0, 1.0, 0.0),
        fov_y: 60.0_f32.to_radians(),
        aspect: 16.0 / 9.0,
        near: 0.1,
        far: 100.0,
    };
    let mut lights = Vec::with_capacity(25);
    for ix in 0..5 {
        for iz in 0..5 {
            let x = (ix as f32 - 2.0) * 1.5;
            let z = -3.0 - (iz as f32) * 1.5;
            // Stagger intensities so accumulation produces distinct
            // image samples.
            let intensity = 0.5 + 0.1 * (ix * 5 + iz) as f32;
            lights.push(Light::point(
                Vec3::new(x, 0.5, z),
                Vec3::new(1.0, 0.95, 0.85),
                intensity,
                3.0,
            ));
        }
    }
    let grid = assign_lights(&cam, &lights);

    // Render the cluster-count heatmap into a 128×72 framebuffer (matches
    // the 16:9 aspect at coarse resolution; one pixel per 0.5 tile so
    // tile boundaries are clearly visible in the reference).
    let mut fb = Framebuffer::new(128, 72);
    clear(&mut fb, Rgba8::default());
    paint_cluster_heatmap(&mut fb, &cam, &grid);

    (
        GoldenScene {
            name: "cluster-lights-128x72",
            framebuffer: fb,
        },
        cam,
        lights,
        grid,
    )
}

/// Fixture: a single directional light + ten box casters. The CSM
/// shadow-heavy reference exercises the four-quadrant atlas + the
/// Vogel-disk PCF. Image: a low-resolution depth-coded slice of the
/// near cascade's atlas quadrant for visual inspection.
pub fn shadow_heavy_scene() -> (GoldenScene, Camera, Vec<Light>, Cascades, ShadowAtlas) {
    let cam = Camera {
        position: Vec3::new(0.0, 4.0, 6.0),
        forward: Vec3::new(0.0, -0.4, -1.0).normalize_or_zero(),
        up: Vec3::new(0.0, 1.0, 0.0),
        fov_y: 60.0_f32.to_radians(),
        aspect: 16.0 / 9.0,
        near: 0.1,
        far: 200.0,
    };
    let light_dir = Vec3::new(-0.3, -1.0, -0.5).normalize_or_zero();
    let lights = vec![Light::directional(
        light_dir,
        Vec3::new(1.0, 0.95, 0.85),
        3.0,
    )];

    let mut casters = Vec::with_capacity(10);
    for i in 0..10 {
        let cx = (i as f32 - 4.5) * 2.0;
        casters.push(MeshInstance {
            aabb: Aabb::from_corners(
                Vec3::new(cx - 0.5, 0.0, -3.0 - (i as f32) * 1.2),
                Vec3::new(cx + 0.5, 1.5, -2.0 - (i as f32) * 1.2),
            ),
            material: Material::grey(),
            casts_shadow: true,
        });
    }

    let cascades = build_cascades(&cam, light_dir, (0.0, 0.0));
    let mut atlas = ShadowAtlas::new();
    render_cascades(&mut atlas, &cascades, &casters);

    // Visualise the first cascade's quadrant at 256×256 (downsampled).
    let mut fb = Framebuffer::new(256, 256);
    clear(&mut fb, Rgba8::default());
    let stride = crate::shadow::CASCADE_DIM / 256;
    for py in 0..256u32 {
        for px in 0..256u32 {
            let ax = cascades.cascades[0].atlas_x + px * stride;
            let ay = cascades.cascades[0].atlas_y + py * stride;
            let d = atlas.read(ax, ay);
            let g = linear_to_srgb_byte(d);
            fb.write(
                px,
                py,
                Rgba8 {
                    r: g,
                    g,
                    b: g,
                    a: 255,
                },
            );
        }
    }

    (
        GoldenScene {
            name: "shadow-heavy-256x256",
            framebuffer: fb,
        },
        cam,
        lights,
        cascades,
        atlas,
    )
}

/// Fixture: a Cook-Torrance lit ground plane sampled per pixel under
/// one directional light + four point lights. The CPU oracle runs the
/// full `accumulate_lighting` per fragment. The `cluster_pixel_parity`
/// test diffs this against the equivalent GPU output (when a GPU
/// runner is available).
pub fn combined_deferred_scene() -> GoldenScene {
    let cam = Camera {
        position: Vec3::new(0.0, 4.0, 6.0),
        forward: Vec3::new(0.0, -0.5, -1.0).normalize_or_zero(),
        up: Vec3::new(0.0, 1.0, 0.0),
        fov_y: 60.0_f32.to_radians(),
        aspect: 16.0 / 9.0,
        near: 0.1,
        far: 100.0,
    };

    let light_dir = Vec3::new(-0.3, -1.0, -0.4).normalize_or_zero();
    let mut lights = vec![Light::directional(
        light_dir,
        Vec3::new(1.0, 0.95, 0.85),
        2.5,
    )];
    for i in 0..4 {
        let angle = (i as f32) * core::f32::consts::TAU / 4.0;
        let x = angle.cos() * 2.5;
        let z = -4.0 + angle.sin() * 2.5;
        let color = match i {
            0 => Vec3::new(1.0, 0.2, 0.2),
            1 => Vec3::new(0.2, 1.0, 0.2),
            2 => Vec3::new(0.2, 0.2, 1.0),
            _ => Vec3::new(1.0, 1.0, 0.2),
        };
        lights.push(Light::point(Vec3::new(x, 1.0, z), color, 4.0, 6.0));
    }

    let casters = vec![MeshInstance {
        aabb: Aabb::from_corners(Vec3::new(-1.0, 0.0, -5.0), Vec3::new(1.0, 2.0, -3.0)),
        material: Material {
            albedo: Vec3::new(0.8, 0.4, 0.2),
            metallic: 0.0,
            roughness: 0.35,
        },
        casts_shadow: true,
    }];

    let grid = assign_lights(&cam, &lights);
    let cascades = build_cascades(&cam, light_dir, (0.0, 0.0));
    let mut atlas = ShadowAtlas::new();
    render_cascades(&mut atlas, &cascades, &casters);

    // Render a 128×72 plane-only image. For each pixel we project a ray
    // through the camera onto the ground plane y=0, evaluate the
    // accumulation oracle, and write the sRGB-encoded result.
    let mut fb = Framebuffer::new(128, 72);
    clear(&mut fb, Rgba8::default());
    let inv_view = cam.view().inverse().unwrap_or(engine_math::Mat4::IDENTITY);
    let inv_proj = cam
        .projection()
        .inverse()
        .unwrap_or(engine_math::Mat4::IDENTITY);

    for py in 0..fb.height() {
        for px in 0..fb.width() {
            let ndc_x = (px as f32 + 0.5) / fb.width() as f32 * 2.0 - 1.0;
            let ndc_y = 1.0 - (py as f32 + 0.5) / fb.height() as f32 * 2.0;
            let clip = Vec4::new(ndc_x, ndc_y, 0.5, 1.0);
            let view_pt4 = inv_proj * clip;
            if view_pt4.w.abs() < 1e-6 {
                continue;
            }
            let view_pt = Vec3::new(
                view_pt4.x / view_pt4.w,
                view_pt4.y / view_pt4.w,
                view_pt4.z / view_pt4.w,
            );
            let world_pt = inv_view.transform_point3(view_pt);
            let ray_dir = Vec3::new(
                world_pt.x - cam.position.x,
                world_pt.y - cam.position.y,
                world_pt.z - cam.position.z,
            )
            .normalize_or_zero();
            // Intersect with ground plane y=0.
            if ray_dir.y.abs() < 1e-4 {
                continue;
            }
            let t = -cam.position.y / ray_dir.y;
            if t <= 0.0 {
                continue;
            }
            let world_p = Vec3::new(
                cam.position.x + ray_dir.x * t,
                0.0,
                cam.position.z + ray_dir.z * t,
            );
            let surface = SurfaceFragment {
                world_p,
                normal: Vec3::new(0.0, 1.0, 0.0),
                material: Material {
                    albedo: Vec3::new(0.55, 0.55, 0.55),
                    metallic: 0.0,
                    roughness: 0.6,
                },
            };
            let radiance = accumulate_lighting(&surface, &cam, &lights, &grid, &cascades, &atlas);
            fb.write(
                px,
                py,
                Rgba8 {
                    r: linear_to_srgb_byte(radiance.x),
                    g: linear_to_srgb_byte(radiance.y),
                    b: linear_to_srgb_byte(radiance.z),
                    a: 255,
                },
            );
        }
    }

    GoldenScene {
        name: "combined-deferred-128x72",
        framebuffer: fb,
    }
}

fn paint_cluster_heatmap(fb: &mut Framebuffer, cam: &Camera, grid: &ClusterGrid) {
    use crate::cluster::{CLUSTER_TILES_X, CLUSTER_TILES_Y, slice_of_view_z};
    let w = fb.width();
    let h = fb.height();
    let inv_view = cam.view().inverse().unwrap_or(engine_math::Mat4::IDENTITY);
    let inv_proj = cam
        .projection()
        .inverse()
        .unwrap_or(engine_math::Mat4::IDENTITY);
    for py in 0..h {
        for px in 0..w {
            let ndc_x = (px as f32 + 0.5) / w as f32 * 2.0 - 1.0;
            let ndc_y = 1.0 - (py as f32 + 0.5) / h as f32 * 2.0;
            let clip = Vec4::new(ndc_x, ndc_y, 0.5, 1.0);
            let view_pt4 = inv_proj * clip;
            if view_pt4.w.abs() < 1e-6 {
                continue;
            }
            let view_pt = Vec3::new(
                view_pt4.x / view_pt4.w,
                view_pt4.y / view_pt4.w,
                view_pt4.z / view_pt4.w,
            );
            // Approximate the cluster the pixel falls into: tile xy
            // from screen position, slice from view-space depth at
            // half-way through the frustum.
            let tx = ((px as f32 / w as f32) * CLUSTER_TILES_X as f32)
                .floor()
                .clamp(0.0, (CLUSTER_TILES_X - 1) as f32) as u32;
            let ty = ((py as f32 / h as f32) * CLUSTER_TILES_Y as f32)
                .floor()
                .clamp(0.0, (CLUSTER_TILES_Y - 1) as f32) as u32;
            let world_pt = inv_view.transform_point3(view_pt);
            let view_z = (cam.view() * Vec4::new(world_pt.x, world_pt.y, world_pt.z, 1.0))
                .z
                .abs();
            let slice = slice_of_view_z(view_z, cam.near, cam.far);
            let count = grid.cell(tx, ty, slice).light_count;
            // Heatmap: blue for 0, green for low, red for high.
            let t = (count as f32 / 8.0).clamp(0.0, 1.0);
            let r = linear_to_srgb_byte(t);
            let g = linear_to_srgb_byte(1.0 - (2.0 * (t - 0.5)).abs());
            let b = linear_to_srgb_byte(1.0 - t);
            fb.write(px, py, Rgba8 { r, g, b, a: 255 });
        }
    }
}

/// Phase 5.5 A.3 — cube parity scene (ADR-046 fixture #1).
///
/// One 1m³ cube at the origin, lit by a single directional light, viewed
/// from `(2, 2, 3)` with a 60° vertical FOV. Both the CPU oracle in this
/// module and the GPU render-graph fixture at
/// `engine-render/tests/pixel_parity/cube.rs` consume this exact scene so
/// the parity check on the two outputs is testing the same inputs through
/// both paths.
///
/// CPU oracle pipeline: ray-march each pixel against the cube AABB,
/// evaluate Cook-Torrance per intersection against the directional light,
/// ACES tonemap, sRGB encode. No IBL, no SSAO, no bloom, no TAA — those
/// are zeroed on the GPU side too (Tonemap's `exposure = 1, bloom_mix = 0`;
/// TAA's `blend_alpha = 1`).
#[derive(Clone, Debug)]
pub struct CubeParityScene {
    /// Camera (view + projection).
    pub camera: Camera,
    /// Single directional light. `position_or_direction` points *toward
    /// the scene*; the surface→light direction is the negation.
    pub light: Light,
    /// Cube bounds in world space.
    pub cube_aabb: Aabb,
    /// Cube material.
    pub material: Material,
    /// Render extent.
    pub width: u32,
    /// Render extent.
    pub height: u32,
}

impl CubeParityScene {
    /// Default v0 scene — 128 × 72, cube at origin, directional sun-like
    /// light from the upper-left, warm-grey albedo at 0.35 roughness.
    pub fn default_v0() -> Self {
        let width = 128u32;
        let height = 72u32;
        Self {
            camera: Camera {
                position: Vec3::new(2.0, 2.0, 3.0),
                forward: Vec3::new(-2.0, -2.0, -3.0).normalize_or_zero(),
                up: Vec3::new(0.0, 1.0, 0.0),
                fov_y: 60.0_f32.to_radians(),
                aspect: width as f32 / height as f32,
                near: 0.1,
                far: 100.0,
            },
            light: Light::directional(Vec3::new(-0.3, -1.0, -0.5), Vec3::new(1.0, 0.95, 0.85), 3.0),
            cube_aabb: Aabb::from_corners(Vec3::new(-0.5, -0.5, -0.5), Vec3::new(0.5, 0.5, 0.5)),
            material: Material {
                albedo: Vec3::new(0.8, 0.4, 0.2),
                metallic: 0.0,
                roughness: 0.35,
            },
            width,
            height,
        }
    }

    /// Render the CPU oracle reference framebuffer.
    pub fn render_cpu(&self) -> Framebuffer {
        let mut fb = Framebuffer::new(self.width, self.height);
        clear(&mut fb, Rgba8::default());
        let inv_view = self.camera.view().inverse().unwrap_or(Mat4::IDENTITY);
        let inv_proj = self.camera.projection().inverse().unwrap_or(Mat4::IDENTITY);

        for py in 0..self.height {
            for px in 0..self.width {
                // Reconstruct a world-space ray from NDC: same algebra as
                // `combined_deferred_scene`, just intersected against the
                // cube AABB rather than a ground plane.
                let ndc_x = (px as f32 + 0.5) / self.width as f32 * 2.0 - 1.0;
                let ndc_y = 1.0 - (py as f32 + 0.5) / self.height as f32 * 2.0;
                let clip = Vec4::new(ndc_x, ndc_y, 0.5, 1.0);
                let view_pt4 = inv_proj * clip;
                if view_pt4.w.abs() < 1e-6 {
                    continue;
                }
                let view_pt = Vec3::new(
                    view_pt4.x / view_pt4.w,
                    view_pt4.y / view_pt4.w,
                    view_pt4.z / view_pt4.w,
                );
                let world_target = inv_view.transform_point3(view_pt);
                let ray_dir = Vec3::new(
                    world_target.x - self.camera.position.x,
                    world_target.y - self.camera.position.y,
                    world_target.z - self.camera.position.z,
                )
                .normalize_or_zero();

                let Some((t, normal)) =
                    intersect_ray_aabb(self.camera.position, ray_dir, &self.cube_aabb)
                else {
                    continue;
                };

                let world_p = Vec3::new(
                    self.camera.position.x + ray_dir.x * t,
                    self.camera.position.y + ray_dir.y * t,
                    self.camera.position.z + ray_dir.z * t,
                );
                let surface = SurfaceFragment {
                    world_p,
                    normal,
                    material: self.material,
                };
                let view_dir = Vec3::new(
                    self.camera.position.x - world_p.x,
                    self.camera.position.y - world_p.y,
                    self.camera.position.z - world_p.z,
                );
                // Directional light: stored direction is "from light to
                // scene"; the BRDF wants "surface to light", so negate.
                let l_to_scene = self.light.position_or_direction;
                let l_dir = Vec3::new(-l_to_scene.x, -l_to_scene.y, -l_to_scene.z);
                let radiance = Vec3::new(
                    self.light.color.x * self.light.intensity,
                    self.light.color.y * self.light.intensity,
                    self.light.color.z * self.light.intensity,
                );
                let lit_linear = cook_torrance(&surface, view_dir, l_dir, radiance);
                let tonemapped = tonemap_aces(lit_linear);
                fb.write(
                    px,
                    py,
                    Rgba8 {
                        r: linear_to_srgb_byte(tonemapped.x),
                        g: linear_to_srgb_byte(tonemapped.y),
                        b: linear_to_srgb_byte(tonemapped.z),
                        a: 255,
                    },
                );
            }
        }
        fb
    }
}

/// Slab-method ray vs. AABB intersection. Returns `(t, normal)` for the
/// nearest entry point, where `t > 0` and `normal` is the unit outward
/// face normal.
fn intersect_ray_aabb(origin: Vec3, dir: Vec3, aabb: &Aabb) -> Option<(f32, Vec3)> {
    let inv = Vec3::new(1.0 / dir.x, 1.0 / dir.y, 1.0 / dir.z);
    let (tx_min, tx_max, nx_pos) = if inv.x >= 0.0 {
        (
            (aabb.min.x - origin.x) * inv.x,
            (aabb.max.x - origin.x) * inv.x,
            Vec3::new(-1.0, 0.0, 0.0),
        )
    } else {
        (
            (aabb.max.x - origin.x) * inv.x,
            (aabb.min.x - origin.x) * inv.x,
            Vec3::new(1.0, 0.0, 0.0),
        )
    };
    let (ty_min, ty_max, ny_pos) = if inv.y >= 0.0 {
        (
            (aabb.min.y - origin.y) * inv.y,
            (aabb.max.y - origin.y) * inv.y,
            Vec3::new(0.0, -1.0, 0.0),
        )
    } else {
        (
            (aabb.max.y - origin.y) * inv.y,
            (aabb.min.y - origin.y) * inv.y,
            Vec3::new(0.0, 1.0, 0.0),
        )
    };
    let (tz_min, tz_max, nz_pos) = if inv.z >= 0.0 {
        (
            (aabb.min.z - origin.z) * inv.z,
            (aabb.max.z - origin.z) * inv.z,
            Vec3::new(0.0, 0.0, -1.0),
        )
    } else {
        (
            (aabb.max.z - origin.z) * inv.z,
            (aabb.min.z - origin.z) * inv.z,
            Vec3::new(0.0, 0.0, 1.0),
        )
    };
    let mut tmin = tx_min;
    let mut normal = nx_pos;
    if ty_min > tmin {
        tmin = ty_min;
        normal = ny_pos;
    }
    if tz_min > tmin {
        tmin = tz_min;
        normal = nz_pos;
    }
    let tmax = tx_max.min(ty_max).min(tz_max);
    if tmax < 0.0 || tmin > tmax || tmin < 0.0 {
        return None;
    }
    Some((tmin, normal))
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
        assert!(
            centre.r > 50 || centre.g > 50,
            "centre is black: {centre:?}"
        );
    }

    #[test]
    fn cluster_lights_scene_populates_grid_with_25_lights() {
        let (_scene, _cam, lights, grid) = cluster_lights_scene();
        assert_eq!(lights.len(), 25);
        // At least one cluster cell must have a non-zero light count.
        assert!(grid.cells().iter().any(|c| c.light_count > 0));
    }

    #[test]
    fn shadow_heavy_scene_writes_into_first_cascade_quadrant() {
        let (_scene, _cam, _lights, cascades, atlas) = shadow_heavy_scene();
        let ox = cascades.cascades[0].atlas_x;
        let oy = cascades.cascades[0].atlas_y;
        let mut any = false;
        for dy in 0..crate::shadow::CASCADE_DIM / 32 {
            for dx in 0..crate::shadow::CASCADE_DIM / 32 {
                if atlas.read(ox + dx * 32, oy + dy * 32) > 0.0 {
                    any = true;
                    break;
                }
            }
            if any {
                break;
            }
        }
        assert!(any, "shadow-heavy scene must populate the first cascade");
    }

    #[test]
    fn combined_deferred_scene_produces_nonzero_image() {
        let scene = combined_deferred_scene();
        // At least one pixel must be lit (the directional light alone
        // would be enough to illuminate the ground plane).
        let any_lit = scene
            .framebuffer
            .color()
            .iter()
            .any(|p| p.r > 0 || p.g > 0 || p.b > 0);
        assert!(any_lit, "combined-deferred scene rendered fully black");
    }

    #[test]
    fn cube_parity_scene_renders_lit_cube() {
        let scene = CubeParityScene::default_v0();
        let fb = scene.render_cpu();
        assert_eq!(fb.width(), scene.width);
        assert_eq!(fb.height(), scene.height);
        // Cube occupies the centre of the framebuffer at this camera
        // angle. Centre pixel must be lit by the warm directional light;
        // an unlit ray miss leaves a black pixel via `Rgba8::default`.
        let centre = fb.sample(scene.width / 2, scene.height / 2);
        assert!(
            centre.r > 0 || centre.g > 0 || centre.b > 0,
            "centre of cube must be lit: {centre:?}"
        );
    }
}
