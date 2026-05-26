//! Cook-Torrance lighting accumulation · CPU oracle reference.
//!
//! The deferred lighting pass on the GPU (`draw.opaque.2`, ADR-043 §5)
//! reads the G-buffer, looks up the cluster cell, dereferences each
//! light, samples the shadow atlas via [`crate::shadow::sample_shadow_pcf`],
//! and accumulates Cook-Torrance BRDF responses. This module is the
//! CPU oracle for that pass.
//!
//! BRDF: Cook-Torrance with GGX/Trowbridge-Reitz microfacet
//! distribution, Smith-Schlick geometry, Schlick Fresnel — the
//! industry-standard PBR baseline. IBL (ADR-041) is *not* applied here;
//! that lands in PR 4.

use crate::cluster::{CLUSTER_TILES_X, CLUSTER_TILES_Y, ClusterGrid, cell_index, slice_of_view_z};
use crate::scene::{Camera, Light, LightType, Material};
use crate::shadow::{Cascades, ShadowAtlas, sample_shadow_pcf, select_cascade};
use engine_math::Vec3;

const PI: f32 = core::f32::consts::PI;

/// One opaque surface fragment, as the deferred G-buffer would produce.
#[derive(Clone, Copy, Debug)]
pub struct SurfaceFragment {
    /// World-space position of the surface point.
    pub world_p: Vec3,
    /// Unit world-space normal.
    pub normal: Vec3,
    /// Material parameters (albedo + metallic + roughness).
    pub material: Material,
}

/// Evaluate Cook-Torrance BRDF for one light at `surface`. Returns the
/// outgoing radiance for the (light, surface, view) triple.
pub fn cook_torrance(
    surface: &SurfaceFragment,
    view_dir: Vec3,
    light_dir: Vec3,
    light_radiance: Vec3,
) -> Vec3 {
    let n = surface.normal.normalize_or_zero();
    let v = view_dir.normalize_or_zero();
    let l = light_dir.normalize_or_zero();
    let n_dot_l = n.dot(l).max(0.0);
    if n_dot_l <= 0.0 {
        return Vec3::new(0.0, 0.0, 0.0);
    }
    let h = Vec3::new(v.x + l.x, v.y + l.y, v.z + l.z).normalize_or_zero();
    let n_dot_v = n.dot(v).max(1e-4);
    let n_dot_h = n.dot(h).max(0.0);
    let v_dot_h = v.dot(h).max(0.0);

    // Schlick Fresnel.
    let f0 = lerp_vec3(
        Vec3::new(0.04, 0.04, 0.04),
        surface.material.albedo,
        surface.material.metallic,
    );
    let one_minus_vdh = 1.0 - v_dot_h;
    let fresnel = vec3_add(
        f0,
        vec3_scale(
            vec3_sub(Vec3::new(1.0, 1.0, 1.0), f0),
            one_minus_vdh.powi(5),
        ),
    );

    // GGX / Trowbridge-Reitz NDF.
    let alpha = surface.material.roughness * surface.material.roughness;
    let alpha2 = (alpha * alpha).max(1e-6);
    let d_denom_root = n_dot_h * n_dot_h * (alpha2 - 1.0) + 1.0;
    let ndf = alpha2 / (PI * d_denom_root * d_denom_root).max(1e-6);

    // Smith-Schlick geometry term.
    let k = (alpha + 1.0) * (alpha + 1.0) / 8.0;
    let g_v = n_dot_v / (n_dot_v * (1.0 - k) + k).max(1e-6);
    let g_l = n_dot_l / (n_dot_l * (1.0 - k) + k).max(1e-6);
    let geom = g_v * g_l;

    let specular_num = vec3_scale(fresnel, ndf * geom);
    let specular_denom = (4.0 * n_dot_v * n_dot_l).max(1e-6);
    let specular = vec3_scale(specular_num, 1.0 / specular_denom);

    // Lambertian diffuse — dielectrics-only. The "energy conservation"
    // term (1 - F) * (1 - metallic) couples diffuse to the Fresnel.
    let kd = vec3_scale(
        vec3_sub(Vec3::new(1.0, 1.0, 1.0), fresnel),
        1.0 - surface.material.metallic,
    );
    let diffuse = vec3_scale(vec3_componentwise(kd, surface.material.albedo), 1.0 / PI);

    let brdf = vec3_add(diffuse, specular);
    vec3_componentwise(vec3_scale(brdf, n_dot_l), light_radiance)
}

/// View-space depth of `world_p` for `cam`. Positive in front of the
/// camera.
pub fn view_space_depth(cam: &Camera, world_p: Vec3) -> f32 {
    let v = cam.view();
    let pt = v * engine_math::Vec4::new(world_p.x, world_p.y, world_p.z, 1.0);
    -pt.z
}

/// Map a world-space point through the camera view-projection and
/// return its screen-space tile (`tile_x`, `tile_y`) within
/// `CLUSTER_TILES_X × CLUSTER_TILES_Y`. Clamps to the grid bounds.
pub fn screen_tile(cam: &Camera, world_p: Vec3) -> (u32, u32) {
    let clip = cam.view_projection() * engine_math::Vec4::new(world_p.x, world_p.y, world_p.z, 1.0);
    if clip.w.abs() < 1e-6 {
        return (0, 0);
    }
    let nx = (clip.x / clip.w) * 0.5 + 0.5;
    let ny = 1.0 - ((clip.y / clip.w) * 0.5 + 0.5);
    let tx = (nx * CLUSTER_TILES_X as f32)
        .floor()
        .clamp(0.0, CLUSTER_TILES_X as f32 - 1.0) as u32;
    let ty = (ny * CLUSTER_TILES_Y as f32)
        .floor()
        .clamp(0.0, CLUSTER_TILES_Y as f32 - 1.0) as u32;
    (tx, ty)
}

/// Accumulate lighting at one surface fragment. Walks the cluster cell
/// the fragment lives in, evaluates Cook-Torrance per light, applies
/// CSM shadow PCF for the light tagged with the shadow atlas, and
/// returns the linear-space radiance.
pub fn accumulate_lighting(
    surface: &SurfaceFragment,
    cam: &Camera,
    lights: &[Light],
    cluster: &ClusterGrid,
    cascades: &Cascades,
    atlas: &ShadowAtlas,
) -> Vec3 {
    let view_dir = Vec3::new(
        cam.position.x - surface.world_p.x,
        cam.position.y - surface.world_p.y,
        cam.position.z - surface.world_p.z,
    );
    let view_z = view_space_depth(cam, surface.world_p);
    let (tx, ty) = screen_tile(cam, surface.world_p);
    let slice = slice_of_view_z(view_z, cam.near, cam.far);
    let cell = cluster.cell(tx, ty, slice);

    let mut color = Vec3::new(0.0, 0.0, 0.0);
    for light_idx in cell.lights() {
        let light = &lights[light_idx as usize];
        let (light_dir, attenuation) = light_dir_and_attenuation(light, surface.world_p);
        if attenuation <= 0.0 {
            continue;
        }
        // Shadow visibility — only the directional light samples the
        // CSM atlas (point-light shadow maps are a Phase 6+ addition).
        let visibility =
            if light.kind == LightType::Directional && light.shadow_atlas_idx != u32::MAX {
                let cascade_idx = select_cascade(cascades, view_z);
                sample_shadow_pcf(atlas, &cascades.cascades[cascade_idx], surface.world_p)
            } else {
                1.0
            };
        let radiance = vec3_scale(light.color, light.intensity * attenuation * visibility);
        let lit = cook_torrance(surface, view_dir, light_dir, radiance);
        color = vec3_add(color, lit);
    }

    // Touch the unused symbol so future code doesn't drift away from the formula.
    let _ = cell_index(0, 0, 0);
    color
}

fn light_dir_and_attenuation(light: &Light, world_p: Vec3) -> (Vec3, f32) {
    match light.kind {
        LightType::Directional => {
            // Light dir points *from* the light toward the scene; the
            // BRDF needs the direction from the surface *toward* the
            // light, so we negate.
            let l_to_scene = light.position_or_direction;
            (Vec3::new(-l_to_scene.x, -l_to_scene.y, -l_to_scene.z), 1.0)
        }
        LightType::Point => {
            let to_l = Vec3::new(
                light.position_or_direction.x - world_p.x,
                light.position_or_direction.y - world_p.y,
                light.position_or_direction.z - world_p.z,
            );
            let dist2 = to_l.length_squared();
            if dist2 >= light.range * light.range {
                return (Vec3::new(0.0, 0.0, 0.0), 0.0);
            }
            let dist = dist2.sqrt().max(1e-3);
            // Inverse-square with a windowed end-of-range falloff.
            let t = (1.0 - dist / light.range).clamp(0.0, 1.0);
            let attenuation = (t * t) / (dist * dist);
            (
                Vec3::new(to_l.x / dist, to_l.y / dist, to_l.z / dist),
                attenuation,
            )
        }
    }
}

#[inline]
fn vec3_add(a: Vec3, b: Vec3) -> Vec3 {
    Vec3::new(a.x + b.x, a.y + b.y, a.z + b.z)
}

#[inline]
fn vec3_sub(a: Vec3, b: Vec3) -> Vec3 {
    Vec3::new(a.x - b.x, a.y - b.y, a.z - b.z)
}

#[inline]
fn vec3_scale(a: Vec3, s: f32) -> Vec3 {
    Vec3::new(a.x * s, a.y * s, a.z * s)
}

#[inline]
fn vec3_componentwise(a: Vec3, b: Vec3) -> Vec3 {
    Vec3::new(a.x * b.x, a.y * b.y, a.z * b.z)
}

#[inline]
fn lerp_vec3(a: Vec3, b: Vec3, t: f32) -> Vec3 {
    Vec3::new(
        a.x + (b.x - a.x) * t,
        a.y + (b.y - a.y) * t,
        a.z + (b.z - a.z) * t,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cook_torrance_grazing_angle_returns_zero() {
        let surface = SurfaceFragment {
            world_p: Vec3::new(0.0, 0.0, 0.0),
            normal: Vec3::new(0.0, 1.0, 0.0),
            material: Material::grey(),
        };
        // Light parallel to the surface: n.dot(l) = 0.
        let out = cook_torrance(
            &surface,
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(1.0, 1.0, 1.0),
        );
        assert_eq!(out, Vec3::new(0.0, 0.0, 0.0));
    }

    #[test]
    fn cook_torrance_normal_incidence_is_finite_positive() {
        let surface = SurfaceFragment {
            world_p: Vec3::new(0.0, 0.0, 0.0),
            normal: Vec3::new(0.0, 1.0, 0.0),
            material: Material {
                albedo: Vec3::new(1.0, 1.0, 1.0),
                metallic: 0.0,
                roughness: 0.5,
            },
        };
        let out = cook_torrance(
            &surface,
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(1.0, 1.0, 1.0),
        );
        assert!(out.x.is_finite() && out.y.is_finite() && out.z.is_finite());
        assert!(out.x > 0.0 && out.y > 0.0 && out.z > 0.0);
    }

    #[test]
    fn metallic_reduces_diffuse_term() {
        let normal_dir = Vec3::new(0.0, 1.0, 0.0);
        let dielectric = SurfaceFragment {
            world_p: Vec3::new(0.0, 0.0, 0.0),
            normal: normal_dir,
            material: Material {
                albedo: Vec3::new(1.0, 0.5, 0.25),
                metallic: 0.0,
                roughness: 0.5,
            },
        };
        let metal = SurfaceFragment {
            world_p: Vec3::new(0.0, 0.0, 0.0),
            normal: normal_dir,
            material: Material {
                albedo: Vec3::new(1.0, 0.5, 0.25),
                metallic: 1.0,
                roughness: 0.5,
            },
        };
        let l = Vec3::new(0.0, 1.0, 0.0);
        let v = Vec3::new(1.0, 1.0, 0.0);
        let radiance = Vec3::new(1.0, 1.0, 1.0);
        let d = cook_torrance(&dielectric, v, l, radiance);
        let m = cook_torrance(&metal, v, l, radiance);
        // Metallic surfaces have no diffuse, so the *blue* channel
        // (lowest albedo) should be smaller for the metallic case.
        assert!(m.z < d.z, "metal blue < dielectric blue: m={m:?} d={d:?}");
    }
}
