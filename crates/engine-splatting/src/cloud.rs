//! `SplatCloud` SoA storage per ADR-077 §2.
//!
//! Cache-friendly parallel-arrays layout: each per-splat attribute is
//! one allocation aligned to a cache line. The sort pass touches
//! `position[]` only; the composite pass touches the appearance
//! arrays. AoS would pull 100+ bytes per splat into cache when the
//! sort needs 12; SoA streams 12 contiguous bytes per splat through
//! L2.

use engine_math::{Quat, Vec3};

/// Number of L=2 SH coefficients per splat per RGB channel: 3 + 5 + 1 = 9.
pub const SH_COEFFS_PER_CHANNEL: usize = 9;

/// SoA storage for an N-splat point cloud.
///
/// Constructed via [`SplatCloudBuilder`] (from the asset-decode path)
/// or [`SplatCloud::from_attributes`] (from the glTF reader). The
/// cloud is immutable after construction; per-frame work reads the
/// arrays but never mutates them.
#[derive(Clone, Debug)]
pub struct SplatCloud {
    position: Vec<Vec3>,
    scale: Vec<Vec3>,
    rotation: Vec<Quat>,
    color: Vec<Vec3>,
    opacity: Vec<f32>,
    sh: Option<Vec<[f32; SH_COEFFS_PER_CHANNEL * 3]>>,
    count: usize,
}

impl SplatCloud {
    /// Construct from owned attribute arrays. All arrays must be the
    /// same length; if `sh.is_some()` it must match too.
    pub fn from_attributes(
        position: Vec<Vec3>,
        scale: Vec<Vec3>,
        rotation: Vec<Quat>,
        color: Vec<Vec3>,
        opacity: Vec<f32>,
        sh: Option<Vec<[f32; SH_COEFFS_PER_CHANNEL * 3]>>,
    ) -> Result<Self, CloudError> {
        let count = position.len();
        if scale.len() != count
            || rotation.len() != count
            || color.len() != count
            || opacity.len() != count
        {
            return Err(CloudError::AttributeLengthMismatch);
        }
        if let Some(sh) = &sh
            && sh.len() != count
        {
            return Err(CloudError::AttributeLengthMismatch);
        }
        Ok(Self {
            position,
            scale,
            rotation,
            color,
            opacity,
            sh,
            count,
        })
    }

    /// Number of splats in the cloud.
    pub fn len(&self) -> usize {
        self.count
    }

    /// True if the cloud has zero splats.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// World-space splat positions.
    pub fn position(&self) -> &[Vec3] {
        &self.position
    }

    /// Per-splat ellipsoid scale (log-space; the GPU shader takes
    /// `exp(scale)` to obtain the linear axes).
    pub fn scale(&self) -> &[Vec3] {
        &self.scale
    }

    /// Per-splat orientation quaternion.
    pub fn rotation(&self) -> &[Quat] {
        &self.rotation
    }

    /// Per-splat base RGB color.
    pub fn color(&self) -> &[Vec3] {
        &self.color
    }

    /// Per-splat alpha (logistic-decoded; in `[0, 1]`).
    pub fn opacity(&self) -> &[f32] {
        &self.opacity
    }

    /// Per-splat L=2 spherical-harmonics coefficients
    /// (3 channels × 9 coefficients = 27 f32 per splat), or `None`
    /// for ambient-only clouds.
    pub fn sh(&self) -> Option<&[[f32; SH_COEFFS_PER_CHANNEL * 3]]> {
        self.sh.as_deref()
    }
}

/// Owned-mutation builder. Used by `engine_splatting::asset::decode`
/// and the glTF reader to construct a [`SplatCloud`] section-by-section.
#[derive(Default)]
pub struct SplatCloudBuilder {
    position: Vec<Vec3>,
    scale: Vec<Vec3>,
    rotation: Vec<Quat>,
    color: Vec<Vec3>,
    opacity: Vec<f32>,
    sh: Option<Vec<[f32; SH_COEFFS_PER_CHANNEL * 3]>>,
}

impl SplatCloudBuilder {
    /// Construct an empty builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Reserve capacity for `n` splats across all arrays.
    pub fn with_capacity(n: usize) -> Self {
        Self {
            position: Vec::with_capacity(n),
            scale: Vec::with_capacity(n),
            rotation: Vec::with_capacity(n),
            color: Vec::with_capacity(n),
            opacity: Vec::with_capacity(n),
            sh: None,
        }
    }

    /// Replace the positions array.
    pub fn positions(mut self, positions: Vec<Vec3>) -> Self {
        self.position = positions;
        self
    }

    /// Replace the scales array.
    pub fn scales(mut self, scales: Vec<Vec3>) -> Self {
        self.scale = scales;
        self
    }

    /// Replace the rotations array.
    pub fn rotations(mut self, rotations: Vec<Quat>) -> Self {
        self.rotation = rotations;
        self
    }

    /// Replace the colors array.
    pub fn colors(mut self, colors: Vec<Vec3>) -> Self {
        self.color = colors;
        self
    }

    /// Replace the opacities array.
    pub fn opacities(mut self, opacities: Vec<f32>) -> Self {
        self.opacity = opacities;
        self
    }

    /// Attach (or replace) the SH coefficients array.
    pub fn spherical_harmonics(
        mut self,
        sh: Vec<[f32; SH_COEFFS_PER_CHANNEL * 3]>,
    ) -> Self {
        self.sh = Some(sh);
        self
    }

    /// Consume the builder, validating attribute lengths.
    pub fn build(self) -> Result<SplatCloud, CloudError> {
        SplatCloud::from_attributes(
            self.position,
            self.scale,
            self.rotation,
            self.color,
            self.opacity,
            self.sh,
        )
    }
}

/// Construction-time error variants.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CloudError {
    /// Two or more attribute arrays had mismatched lengths.
    AttributeLengthMismatch,
}

impl core::fmt::Display for CloudError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            CloudError::AttributeLengthMismatch => {
                write!(f, "splat cloud attribute arrays have mismatched lengths")
            }
        }
    }
}

impl std::error::Error for CloudError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit_quat() -> Quat {
        Quat::IDENTITY
    }

    #[test]
    fn from_attributes_validates_lengths() {
        let n = 4;
        let positions = vec![Vec3::ZERO; n];
        let scales = vec![Vec3::ONE; n];
        let rotations = vec![unit_quat(); n];
        let colors = vec![Vec3::ONE; n];
        let opacities = vec![1.0f32; n];
        let cloud =
            SplatCloud::from_attributes(positions, scales, rotations, colors, opacities, None)
                .expect("valid cloud");
        assert_eq!(cloud.len(), n);
        assert!(!cloud.is_empty());
        assert!(cloud.sh().is_none());
    }

    #[test]
    fn from_attributes_rejects_mismatched_lengths() {
        let err = SplatCloud::from_attributes(
            vec![Vec3::ZERO; 3],
            vec![Vec3::ONE; 4],
            vec![unit_quat(); 3],
            vec![Vec3::ONE; 3],
            vec![1.0; 3],
            None,
        )
        .expect_err("mismatched lengths");
        assert_eq!(err, CloudError::AttributeLengthMismatch);
    }

    #[test]
    fn builder_round_trip() {
        let n = 2;
        let cloud = SplatCloudBuilder::with_capacity(n)
            .positions(vec![Vec3::ZERO; n])
            .scales(vec![Vec3::ONE; n])
            .rotations(vec![unit_quat(); n])
            .colors(vec![Vec3::new(0.5, 0.5, 0.5); n])
            .opacities(vec![0.75; n])
            .build()
            .expect("builds");
        assert_eq!(cloud.len(), n);
        assert_eq!(cloud.opacity()[0], 0.75);
    }

    #[test]
    fn builder_with_sh_array() {
        let n = 1;
        let sh: Vec<[f32; 27]> = vec![[0.0; 27]; n];
        let cloud = SplatCloudBuilder::with_capacity(n)
            .positions(vec![Vec3::ZERO; n])
            .scales(vec![Vec3::ONE; n])
            .rotations(vec![unit_quat(); n])
            .colors(vec![Vec3::ONE; n])
            .opacities(vec![1.0; n])
            .spherical_harmonics(sh)
            .build()
            .expect("builds");
        assert!(cloud.sh().is_some());
        assert_eq!(cloud.sh().unwrap()[0].len(), 27);
    }
}
