//! Scene primitives consumed by the deferred CPU oracle.
//!
//! The CPU rasterizer in this crate is the source-of-truth reference
//! for the GPU pipeline (ADR-046). PR 3's deferred path needs concrete
//! shared input types — cameras, frustums, mesh instances, lights —
//! so the oracle can build a scene, run the cluster + shadow + lighting
//! reference, and emit a [`Framebuffer`](crate::Framebuffer) the GPU
//! output is compared against.
//!
//! All types here are pure data. The math operations live next to the
//! pass that consumes them (`cluster.rs`, `shadow.rs`, `shading.rs`).

use engine_math::{Mat4, Vec3};

/// Pinhole camera with right-handed projection, `[0, 1]` depth range.
///
/// PR 3 keeps the projection convention consistent with ADR-040 / ADR-043:
/// the cluster grid's logarithmic depth slicing is parameterised on
/// `(near, far)`; the CSM cascade splits read them the same way.
#[derive(Clone, Copy, Debug)]
pub struct Camera {
    /// World-space camera position.
    pub position: Vec3,
    /// Forward direction (unit, world-space).
    pub forward: Vec3,
    /// Up direction (unit, world-space).
    pub up: Vec3,
    /// Vertical field of view in radians.
    pub fov_y: f32,
    /// Width / height.
    pub aspect: f32,
    /// Near plane distance (must be > 0).
    pub near: f32,
    /// Far plane distance (must be > near).
    pub far: f32,
}

impl Camera {
    /// View matrix (world → camera). Right-handed look-at.
    pub fn view(&self) -> Mat4 {
        // Right-handed look-at: forward maps to -Z in view space.
        let f = self.forward.normalize_or_zero();
        let s = f.cross(self.up).normalize_or_zero();
        let u = s.cross(f);
        let p = self.position;
        // Column-major rows-of-the-rotation matrix layout, then translate.
        Mat4::from_cols_array([
            s.x,
            u.x,
            -f.x,
            0.0,
            s.y,
            u.y,
            -f.y,
            0.0,
            s.z,
            u.z,
            -f.z,
            0.0,
            -s.dot(p),
            -u.dot(p),
            f.dot(p),
            1.0,
        ])
    }

    /// Projection matrix (camera → clip space, right-handed, `[0, 1]` Z).
    pub fn projection(&self) -> Mat4 {
        Mat4::perspective_rh(self.fov_y, self.aspect, self.near, self.far)
    }

    /// View-projection product.
    pub fn view_projection(&self) -> Mat4 {
        self.projection() * self.view()
    }
}

/// World-space axis-aligned bounding box. Used by frustum culling and
/// the per-instance mesh proxy.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    /// Minimum corner.
    pub min: Vec3,
    /// Maximum corner.
    pub max: Vec3,
}

impl Aabb {
    /// Construct an AABB from two corners (corners are sorted).
    pub fn from_corners(a: Vec3, b: Vec3) -> Self {
        Self {
            min: Vec3::new(a.x.min(b.x), a.y.min(b.y), a.z.min(b.z)),
            max: Vec3::new(a.x.max(b.x), a.y.max(b.y), a.z.max(b.z)),
        }
    }

    /// Centre of the AABB.
    pub fn centre(&self) -> Vec3 {
        Vec3::new(
            (self.min.x + self.max.x) * 0.5,
            (self.min.y + self.max.y) * 0.5,
            (self.min.z + self.max.z) * 0.5,
        )
    }

    /// Half-extents (positive).
    pub fn half_extents(&self) -> Vec3 {
        Vec3::new(
            (self.max.x - self.min.x) * 0.5,
            (self.max.y - self.min.y) * 0.5,
            (self.max.z - self.min.z) * 0.5,
        )
    }

    /// Tightest enclosing sphere — centre + radius.
    pub fn bounding_sphere(&self) -> (Vec3, f32) {
        let c = self.centre();
        let h = self.half_extents();
        let r = (h.x * h.x + h.y * h.y + h.z * h.z).sqrt();
        (c, r)
    }
}

/// Plane in Hessian form: `n.dot(p) + d = 0`. `n` is a unit normal
/// pointing away from the inside half-space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plane {
    /// Unit normal.
    pub normal: Vec3,
    /// Signed offset from origin.
    pub d: f32,
}

impl Plane {
    /// Construct a plane from a unit normal and an offset.
    pub fn new(normal: Vec3, d: f32) -> Self {
        Self { normal, d }
    }

    /// Signed distance from `p` to the plane. Positive on the normal side.
    pub fn signed_distance(&self, p: Vec3) -> f32 {
        self.normal.dot(p) + self.d
    }
}

/// Frustum in world space (six planes; normals point inward).
#[derive(Clone, Copy, Debug)]
pub struct Frustum {
    /// `[near, far, left, right, top, bottom]` — the order is fixed so
    /// tests can name a plane by index.
    pub planes: [Plane; 6],
}

impl Frustum {
    /// Build a frustum from a view-projection matrix using the
    /// Gribb-Hartmann algorithm. The matrix is column-major, the same
    /// convention as `engine_math::Mat4`. Planes are normalised.
    pub fn from_view_projection(vp: Mat4) -> Self {
        let m = vp.to_cols_array();
        // Rows of the transpose, but layout-flat: row[0] = (m00, m10, m20, m30) etc.
        let row = |r: usize| (m[r], m[4 + r], m[8 + r], m[12 + r]);
        let (m00, m10, m20, m30) = row(0);
        let (m01, m11, m21, m31) = row(1);
        let (m02, m12, m22, m32) = row(2);
        let (m03, m13, m23, m33) = row(3);
        // Near (z=0 in `[0,1]` clip): row3 + row2
        // Far  (z=1):                  row3 - row2
        // Left (x=-w):                 row3 + row0
        // Right (x=w):                 row3 - row0
        // Top   (y=w):                 row3 - row1
        // Bottom (y=-w):               row3 + row1
        let raw = [
            (m03 + m02, m13 + m12, m23 + m22, m33 + m32),
            (m03 - m02, m13 - m12, m23 - m22, m33 - m32),
            (m03 + m00, m13 + m10, m23 + m20, m33 + m30),
            (m03 - m00, m13 - m10, m23 - m20, m33 - m30),
            (m03 - m01, m13 - m11, m23 - m21, m33 - m31),
            (m03 + m01, m13 + m11, m23 + m21, m33 + m31),
        ];
        let mut planes = [Plane::new(Vec3::new(1.0, 0.0, 0.0), 0.0); 6];
        for (i, (a, b, c, d)) in raw.into_iter().enumerate() {
            let n = Vec3::new(a, b, c);
            let len = n.length();
            let s = if len > 0.0 { 1.0 / len } else { 1.0 };
            planes[i] = Plane::new(Vec3::new(a * s, b * s, c * s), d * s);
        }
        Self { planes }
    }

    /// Test an AABB against the frustum (conservative reject). Returns
    /// `true` when the AABB is wholly outside any plane; `false` if it
    /// could be visible.
    pub fn rejects_aabb(&self, b: &Aabb) -> bool {
        for plane in &self.planes {
            // p-vertex: the AABB corner farthest along the plane normal.
            let p = Vec3::new(
                if plane.normal.x >= 0.0 {
                    b.max.x
                } else {
                    b.min.x
                },
                if plane.normal.y >= 0.0 {
                    b.max.y
                } else {
                    b.min.y
                },
                if plane.normal.z >= 0.0 {
                    b.max.z
                } else {
                    b.min.z
                },
            );
            if plane.signed_distance(p) < 0.0 {
                return true;
            }
        }
        false
    }
}

/// Material parameters for the CPU oracle's BRDF. Mirrors the
/// per-material data the deferred G-buffer pass writes (albedo,
/// metallic, roughness).
#[derive(Clone, Copy, Debug)]
pub struct Material {
    /// sRGB-decoded linear albedo.
    pub albedo: Vec3,
    /// 0 = dielectric, 1 = metal.
    pub metallic: f32,
    /// 0 = mirror, 1 = fully rough.
    pub roughness: f32,
}

impl Material {
    /// Default lambertian-grey material.
    pub fn grey() -> Self {
        Self {
            albedo: Vec3::new(0.7, 0.7, 0.7),
            metallic: 0.0,
            roughness: 0.5,
        }
    }
}

/// Single drawable in the CPU oracle scene: an axis-aligned box with
/// a material. PR 3's oracle does not need general meshes; the fixtures
/// are box-built scenes.
#[derive(Clone, Copy, Debug)]
pub struct MeshInstance {
    /// World-space AABB.
    pub aabb: Aabb,
    /// Surface material.
    pub material: Material,
    /// `true` if the instance casts shadows. The shadow pass culls on
    /// this flag.
    pub casts_shadow: bool,
}

/// Light kind. `Directional` is the sun-like light; `Point` is the
/// most common cluster-bin candidate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LightType {
    /// Distant, parallel rays (e.g. sun).
    Directional,
    /// Omnidirectional point light with range falloff.
    Point,
}

/// Single GPU-compatible light record. The shape matches the SSBO
/// layout described in ADR-043 §3 (`GpuLight`), but here it is the
/// CPU-side reference; the GPU layout adds explicit padding which the
/// engine-gpu pass-resource helpers will own.
#[derive(Clone, Copy, Debug)]
pub struct Light {
    /// Light kind.
    pub kind: LightType,
    /// Position (`Point`) or direction (`Directional`, unit vector
    /// pointing *from* the light toward the scene).
    pub position_or_direction: Vec3,
    /// Linear RGB colour.
    pub color: Vec3,
    /// Lumen-class intensity multiplier (the cluster pass uses this).
    pub intensity: f32,
    /// Range in metres (`Point` only; `Directional` ignores this).
    pub range: f32,
    /// Index into the cascade atlas if this light casts shadow, else
    /// [`u32::MAX`].
    pub shadow_atlas_idx: u32,
}

impl Light {
    /// Build a directional sun light.
    pub fn directional(direction: Vec3, color: Vec3, intensity: f32) -> Self {
        Self {
            kind: LightType::Directional,
            position_or_direction: direction.normalize_or_zero(),
            color,
            intensity,
            range: f32::INFINITY,
            shadow_atlas_idx: 0,
        }
    }

    /// Build a point light.
    pub fn point(position: Vec3, color: Vec3, intensity: f32, range: f32) -> Self {
        Self {
            kind: LightType::Point,
            position_or_direction: position,
            color,
            intensity,
            range,
            shadow_atlas_idx: u32::MAX,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aabb_bounding_sphere_radius_matches_corner_distance() {
        let b = Aabb::from_corners(Vec3::new(-1.0, -2.0, -3.0), Vec3::new(1.0, 2.0, 3.0));
        let (c, r) = b.bounding_sphere();
        assert_eq!(c, Vec3::new(0.0, 0.0, 0.0));
        assert!((r - (1.0f32 + 4.0 + 9.0).sqrt()).abs() < 1e-5);
    }

    #[test]
    fn frustum_rejects_far_box() {
        let cam = Camera {
            position: Vec3::new(0.0, 0.0, 0.0),
            forward: Vec3::new(0.0, 0.0, -1.0),
            up: Vec3::new(0.0, 1.0, 0.0),
            fov_y: 1.0,
            aspect: 1.0,
            near: 0.1,
            far: 100.0,
        };
        let vp = cam.view_projection();
        let f = Frustum::from_view_projection(vp);
        // A box behind the camera.
        let b = Aabb::from_corners(Vec3::new(-1.0, -1.0, 10.0), Vec3::new(1.0, 1.0, 12.0));
        assert!(f.rejects_aabb(&b), "behind-camera box must be rejected");
        // A box in front, on-axis.
        let inside = Aabb::from_corners(Vec3::new(-0.5, -0.5, -5.0), Vec3::new(0.5, 0.5, -4.0));
        assert!(!f.rejects_aabb(&inside));
    }

    #[test]
    fn plane_signed_distance_is_signed() {
        let p = Plane::new(Vec3::new(1.0, 0.0, 0.0), -1.0);
        assert!((p.signed_distance(Vec3::new(2.0, 0.0, 0.0)) - 1.0).abs() < 1e-5);
        assert!((p.signed_distance(Vec3::new(0.0, 0.0, 0.0)) + 1.0).abs() < 1e-5);
    }
}
