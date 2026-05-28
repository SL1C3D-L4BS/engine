//! glTF KHR_gaussian_splatting extension reader (ADR-078 §5).
//!
//! Consumed *only* by the `tools/engine-splat-import/` subprocess —
//! the engine binary never re-parses glTF at runtime. The reader
//! maps the extension's attribute layout (interleaved or
//! non-interleaved accessor streams) into the SoA shape the
//! engine's ESPL format expects.
//!
//! The Khronos draft (August 2025) is not yet ratified; this reader
//! pins a specific draft revision and refuses files that name a
//! different rev. The actual byte-level glTF parsing is the caller's
//! responsibility (typically via the `gltf` crate in the import
//! subprocess); this module accepts already-parsed attribute streams
//! and produces a [`SplatCloud`].

use crate::cloud::{SH_COEFFS_PER_CHANNEL, SplatCloud, SplatCloudBuilder};
use engine_math::{Quat, Vec3};

/// The KHR_gaussian_splatting draft revision this reader supports.
/// Files naming a different rev are refused via [`GltfError::UnsupportedRevision`].
pub const KHR_GAUSSIAN_SPLATTING_DRAFT_REV: &str = "draft-2025-08";

/// The five extension-defined attribute names (per draft 2025-08).
pub mod attribute {
    /// Per-splat world-space position. Vec3 f32.
    pub const POSITION: &str = "POSITION";
    /// Per-splat ellipsoid scale (log-space). Vec3 f32.
    pub const SCALE: &str = "_SCALE";
    /// Per-splat orientation quaternion. Vec4 f32 (x, y, z, w).
    pub const ROTATION: &str = "_ROTATION";
    /// Per-splat base color (and alpha in the .w slot). Vec4 f32.
    pub const COLOR_0: &str = "COLOR_0";
    /// Per-splat L=2 SH coefficients. 27 × f32 (channels × bands).
    pub const SPHERICAL_HARMONICS: &str = "_SPHERICAL_HARMONICS";
}

/// Read-side input: already-extracted attribute streams from the
/// caller's glTF parse. Each stream is `Vec<f32>` of expected length
/// (N for scalars, N×3 for Vec3, N×4 for Quat, N×27 for SH).
pub struct GltfAttributes {
    /// Caller-declared extension revision (must match
    /// [`KHR_GAUSSIAN_SPLATTING_DRAFT_REV`]).
    pub revision: String,
    /// N × 3 — positions, flattened.
    pub positions: Vec<f32>,
    /// N × 3 — scales (log-space), flattened.
    pub scales: Vec<f32>,
    /// N × 4 — rotations as quaternions, flattened.
    pub rotations: Vec<f32>,
    /// N × 4 — colors (rgba), flattened. The alpha channel becomes
    /// the per-splat opacity.
    pub colors: Vec<f32>,
    /// Optional: N × 27 — SH coefficients, flattened.
    pub spherical_harmonics: Option<Vec<f32>>,
}

/// Reader error variants.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GltfError {
    /// Extension revision is not [`KHR_GAUSSIAN_SPLATTING_DRAFT_REV`].
    UnsupportedRevision(String),
    /// Attribute stream's length is not the expected splat-count-derived value.
    AttributeLengthMismatch {
        /// Attribute name that was wrong.
        attribute: &'static str,
        /// Expected stream length (f32 count).
        expected: usize,
        /// Actual stream length (f32 count).
        actual: usize,
    },
}

impl core::fmt::Display for GltfError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            GltfError::UnsupportedRevision(r) => write!(
                f,
                "KHR_gaussian_splatting revision mismatch: file says {r:?}, reader supports {KHR_GAUSSIAN_SPLATTING_DRAFT_REV:?}",
            ),
            GltfError::AttributeLengthMismatch {
                attribute,
                expected,
                actual,
            } => write!(
                f,
                "{attribute}: expected {expected} f32 values, got {actual}"
            ),
        }
    }
}

impl std::error::Error for GltfError {}

/// Read the parsed attribute streams into a [`SplatCloud`].
///
/// The splat count is derived from the positions stream length /
/// 3. All other streams are length-validated against it.
pub fn read(attrs: GltfAttributes) -> Result<SplatCloud, GltfError> {
    if attrs.revision != KHR_GAUSSIAN_SPLATTING_DRAFT_REV {
        return Err(GltfError::UnsupportedRevision(attrs.revision));
    }
    if attrs.positions.len() % 3 != 0 {
        return Err(GltfError::AttributeLengthMismatch {
            attribute: attribute::POSITION,
            expected: 3 * (attrs.positions.len() / 3),
            actual: attrs.positions.len(),
        });
    }
    let n = attrs.positions.len() / 3;

    if attrs.scales.len() != n * 3 {
        return Err(GltfError::AttributeLengthMismatch {
            attribute: attribute::SCALE,
            expected: n * 3,
            actual: attrs.scales.len(),
        });
    }
    if attrs.rotations.len() != n * 4 {
        return Err(GltfError::AttributeLengthMismatch {
            attribute: attribute::ROTATION,
            expected: n * 4,
            actual: attrs.rotations.len(),
        });
    }
    if attrs.colors.len() != n * 4 {
        return Err(GltfError::AttributeLengthMismatch {
            attribute: attribute::COLOR_0,
            expected: n * 4,
            actual: attrs.colors.len(),
        });
    }

    let mut positions = Vec::with_capacity(n);
    let mut scales = Vec::with_capacity(n);
    let mut rotations = Vec::with_capacity(n);
    let mut colors = Vec::with_capacity(n);
    let mut opacities = Vec::with_capacity(n);

    for i in 0..n {
        let p = &attrs.positions[i * 3..i * 3 + 3];
        positions.push(Vec3::new(p[0], p[1], p[2]));
        let s = &attrs.scales[i * 3..i * 3 + 3];
        scales.push(Vec3::new(s[0], s[1], s[2]));
        let q = &attrs.rotations[i * 4..i * 4 + 4];
        rotations.push(Quat::new(q[0], q[1], q[2], q[3]));
        let c = &attrs.colors[i * 4..i * 4 + 4];
        colors.push(Vec3::new(c[0], c[1], c[2]));
        opacities.push(c[3]);
    }

    let builder = SplatCloudBuilder::with_capacity(n)
        .positions(positions)
        .scales(scales)
        .rotations(rotations)
        .colors(colors)
        .opacities(opacities);

    let builder = if let Some(sh) = attrs.spherical_harmonics {
        let needed = n * SH_COEFFS_PER_CHANNEL * 3;
        if sh.len() != needed {
            return Err(GltfError::AttributeLengthMismatch {
                attribute: attribute::SPHERICAL_HARMONICS,
                expected: needed,
                actual: sh.len(),
            });
        }
        let mut sh_arrays = Vec::with_capacity(n);
        for i in 0..n {
            let mut splat_sh = [0.0f32; SH_COEFFS_PER_CHANNEL * 3];
            for k in 0..(SH_COEFFS_PER_CHANNEL * 3) {
                splat_sh[k] = sh[i * SH_COEFFS_PER_CHANNEL * 3 + k];
            }
            sh_arrays.push(splat_sh);
        }
        builder.spherical_harmonics(sh_arrays)
    } else {
        builder
    };

    builder.build().map_err(|_| GltfError::AttributeLengthMismatch {
        attribute: "internal",
        expected: 0,
        actual: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn smoke_attrs(n: usize) -> GltfAttributes {
        GltfAttributes {
            revision: KHR_GAUSSIAN_SPLATTING_DRAFT_REV.to_string(),
            positions: vec![0.0; n * 3],
            scales: vec![1.0; n * 3],
            rotations: {
                let mut v = vec![0.0f32; n * 4];
                for i in 0..n {
                    v[i * 4 + 3] = 1.0; // identity quat
                }
                v
            },
            colors: vec![0.5; n * 4],
            spherical_harmonics: None,
        }
    }

    #[test]
    fn read_minimal_cloud() {
        let cloud = read(smoke_attrs(2)).expect("reads");
        assert_eq!(cloud.len(), 2);
    }

    #[test]
    fn rejects_wrong_revision() {
        let mut a = smoke_attrs(1);
        a.revision = "draft-1999-12".to_string();
        let err = read(a).unwrap_err();
        assert!(matches!(err, GltfError::UnsupportedRevision(_)));
    }

    #[test]
    fn rejects_mismatched_attribute_length() {
        let mut a = smoke_attrs(2);
        a.scales.pop(); // make scales 5 floats instead of 6
        let err = read(a).unwrap_err();
        assert!(matches!(err, GltfError::AttributeLengthMismatch { .. }));
    }

    #[test]
    fn round_trip_with_sh() {
        let n = 2;
        let mut a = smoke_attrs(n);
        a.spherical_harmonics = Some(vec![0.0; n * 27]);
        let cloud = read(a).expect("reads");
        assert!(cloud.sh().is_some());
        assert_eq!(cloud.sh().unwrap().len(), n);
    }
}
