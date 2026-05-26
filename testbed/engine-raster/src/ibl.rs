//! Image-based lighting · L2 SH probe oracle (ADR-041).
//!
//! Probes are sampled on a sparse, world-cell grid (default cell size
//! 4 m). Each probe stores nine RGB L2 SH coefficients of the incident
//! radiance field. The lighting pass samples the eight nearest grid
//! neighbours of a fragment's world position and trilinearly
//! interpolates their coefficients before evaluating the cosine-lobe
//! convolution at the surface normal.
//!
//! The closed-form irradiance for a directional light reconstructed
//! through L2 SH (Ramamoorthi & Hanrahan 2001) is the verification
//! oracle — see [`ShL2::from_directional_light`] and the unit tests.

use engine_math::Vec3;

const PI: f32 = core::f32::consts::PI;

/// Cosine-lobe convolution weight for band l = 0 (Ramamoorthi-Hanrahan).
pub const SH_A0: f32 = PI;
/// Cosine-lobe convolution weight for band l = 1.
pub const SH_A1: f32 = 2.0 * PI / 3.0;
/// Cosine-lobe convolution weight for band l = 2.
pub const SH_A2: f32 = PI / 4.0;

// Real-valued L2 SH basis normalisation constants (Condon-Shortley
// convention; same as RTR4 §10.3.2 / Sloan 2008 §6).
const SH_Y00: f32 = 0.282_094_8; // 0.5 / sqrt(pi)
const SH_Y1: f32 = 0.488_602_5; // sqrt(3 / (4*pi))
const SH_Y2_XY: f32 = 1.092_548; // sqrt(15 / (4*pi))
const SH_Y20: f32 = 0.315_392; // 0.25 * sqrt(5/pi)
const SH_Y22: f32 = 0.546_274; // 0.25 * sqrt(15/pi)

/// Default world-space cell size for the probe grid (ADR-041 §2).
pub const PROBE_CELL_SIZE: f32 = 4.0;
/// Maximum probe count per scene (ADR-041 §2 / spec §IV.4.A line 382).
pub const MAX_PROBES: usize = 128;

/// Nine RGB SH coefficients (L2 basis). Stores the SH expansion of an
/// incident radiance field; pass it through [`ShL2::evaluate_irradiance`]
/// to recover the cosine-convolved irradiance at a surface normal.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShL2 {
    /// Nine bands in `(l, m)` order: `(0,0)`, `(1,-1)`, `(1,0)`, `(1,1)`,
    /// `(2,-2)`, `(2,-1)`, `(2,0)`, `(2,1)`, `(2,2)`.
    pub coeffs: [Vec3; 9],
}

impl ShL2 {
    /// All-zero coefficients.
    pub const ZERO: Self = Self {
        coeffs: [Vec3::ZERO; 9],
    };

    /// Build directly from raw band coefficients.
    pub const fn new(coeffs: [Vec3; 9]) -> Self {
        Self { coeffs }
    }

    /// Real SH basis `Y_lm(d)` evaluated at unit direction `d`. Returns
    /// nine scalars in band-major order.
    pub fn basis(d: Vec3) -> [f32; 9] {
        let (x, y, z) = (d.x, d.y, d.z);
        [
            SH_Y00,
            -SH_Y1 * y,
            SH_Y1 * z,
            -SH_Y1 * x,
            SH_Y2_XY * x * y,
            -SH_Y2_XY * y * z,
            SH_Y20 * (3.0 * z * z - 1.0),
            -SH_Y2_XY * x * z,
            SH_Y22 * (x * x - y * y),
        ]
    }

    /// Per-band cosine-lobe convolution weights `A_l`.
    pub const fn cosine_lobe_weights() -> [f32; 9] {
        [
            SH_A0, SH_A1, SH_A1, SH_A1, SH_A2, SH_A2, SH_A2, SH_A2, SH_A2,
        ]
    }

    /// Build the SH expansion of a directional light. `direction` is the
    /// unit vector pointing *toward* the light from the surface; `color`
    /// is the light's emitted radiance.
    pub fn from_directional_light(direction: Vec3, color: Vec3) -> Self {
        let d = direction.normalize_or_zero();
        let basis = Self::basis(d);
        let mut coeffs = [Vec3::ZERO; 9];
        for (i, c) in coeffs.iter_mut().enumerate() {
            let s = basis[i];
            *c = Vec3::new(color.x * s, color.y * s, color.z * s);
        }
        Self { coeffs }
    }

    /// Build the SH expansion of a constant ambient sky. `radiance` is
    /// the per-channel radiance integrated over the sphere.
    pub fn from_ambient(radiance: Vec3) -> Self {
        // Only the DC band is non-zero for a constant signal. The DC
        // coefficient of a uniform sphere of radiance L is L / Y_00 ·
        // (1 / 4π) · 4π = L / Y_00 · 1 → integrate Y_00 over the sphere:
        // ∫ Y_00 dΩ = sqrt(4π), so L_00 = L · sqrt(4π).
        // Equivalent factor: 1 / Y_00 = 4π · Y_00.
        let scale = 1.0 / SH_Y00;
        let mut coeffs = [Vec3::ZERO; 9];
        coeffs[0] = Vec3::new(radiance.x * scale, radiance.y * scale, radiance.z * scale);
        Self { coeffs }
    }

    /// Evaluate cosine-convolved irradiance at unit surface normal `n`.
    /// Negative components (the L2 truncation can leak below zero on
    /// the dark side) are clamped to zero.
    pub fn evaluate_irradiance(&self, n: Vec3) -> Vec3 {
        let basis = Self::basis(n.normalize_or_zero());
        let weights = Self::cosine_lobe_weights();
        let mut acc = Vec3::ZERO;
        for i in 0..9 {
            let w = weights[i] * basis[i];
            acc = Vec3::new(
                acc.x + self.coeffs[i].x * w,
                acc.y + self.coeffs[i].y * w,
                acc.z + self.coeffs[i].z * w,
            );
        }
        Vec3::new(acc.x.max(0.0), acc.y.max(0.0), acc.z.max(0.0))
    }

    /// Componentwise scalar multiply.
    pub fn scale(&self, s: f32) -> Self {
        let mut out = [Vec3::ZERO; 9];
        for (i, slot) in out.iter_mut().enumerate() {
            *slot = Vec3::new(
                self.coeffs[i].x * s,
                self.coeffs[i].y * s,
                self.coeffs[i].z * s,
            );
        }
        Self { coeffs: out }
    }

    /// Componentwise sum.
    pub fn add(&self, other: &Self) -> Self {
        let mut out = [Vec3::ZERO; 9];
        for (i, slot) in out.iter_mut().enumerate() {
            *slot = Vec3::new(
                self.coeffs[i].x + other.coeffs[i].x,
                self.coeffs[i].y + other.coeffs[i].y,
                self.coeffs[i].z + other.coeffs[i].z,
            );
        }
        Self { coeffs: out }
    }
}

/// World-space cell key. Probes are bucketed onto a uniform 3D grid
/// keyed by `(floor(x / cell_size), floor(y / cell_size), floor(z / cell_size))`.
pub type CellKey = (i16, i16, i16);

/// A single probe record: world-space position + SH coefficients of the
/// incident radiance field captured at that point.
#[derive(Clone, Copy, Debug)]
pub struct Probe {
    /// World-space position of the probe centre.
    pub position: Vec3,
    /// L2 SH coefficients of the incident radiance.
    pub sh: ShL2,
}

/// Sparse 3D grid of L2 SH probes (ADR-041). Probes are bucketed onto
/// a uniform grid by world-space cell; missing cells fall back to a
/// single global probe (a neutral-ambient default in Phase 5 per
/// ADR-041 §2).
///
/// The container stores entries in `CellKey`-sorted order so iteration
/// and serialisation are deterministic across runs. The runtime
/// engine-render variant uses
/// `engine_core::collections::HashMap<CellKey, ProbeId>` with the
/// `DeterministicHasher` — the same sort-then-build ordering reduces
/// to the same set of `(key, probe)` records, and the CPU oracle's
/// trilinear lookup is set-identical to the GPU version.
#[derive(Clone, Debug)]
pub struct IblProbeSet {
    /// Cell edge length in metres.
    pub cell_size: f32,
    /// Fallback probe used wherever the grid is empty.
    pub fallback: ShL2,
    entries: Vec<(CellKey, Probe)>,
}

impl IblProbeSet {
    /// Allocate an empty probe set with the given cell size and
    /// fallback. Cell sizes must be strictly positive.
    pub fn new(cell_size: f32, fallback: ShL2) -> Self {
        debug_assert!(cell_size > 0.0);
        Self {
            cell_size,
            fallback,
            entries: Vec::new(),
        }
    }

    /// Number of probes registered.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True if no probes are registered.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Borrow the sorted `(cell, probe)` entries.
    pub fn entries(&self) -> &[(CellKey, Probe)] {
        &self.entries
    }

    /// Compute the cell key for a world-space position under this
    /// probe set's cell size.
    pub fn cell_key(&self, world_pos: Vec3) -> CellKey {
        let cx = (world_pos.x / self.cell_size).floor() as i16;
        let cy = (world_pos.y / self.cell_size).floor() as i16;
        let cz = (world_pos.z / self.cell_size).floor() as i16;
        (cx, cy, cz)
    }

    /// Insert or replace the probe at `probe.position`. Returns `true`
    /// on success, `false` when the [`MAX_PROBES`] cap is hit and the
    /// probe is being added to a new cell.
    pub fn insert(&mut self, probe: Probe) -> bool {
        let key = self.cell_key(probe.position);
        match self.entries.binary_search_by_key(&key, |(k, _)| *k) {
            Ok(i) => {
                self.entries[i].1 = probe;
                true
            }
            Err(i) => {
                if self.entries.len() >= MAX_PROBES {
                    return false;
                }
                self.entries.insert(i, (key, probe));
                true
            }
        }
    }

    /// Look up the probe occupying `key`, or `None` if the cell is empty.
    pub fn probe_at(&self, key: CellKey) -> Option<&Probe> {
        self.entries
            .binary_search_by_key(&key, |(k, _)| *k)
            .ok()
            .map(|i| &self.entries[i].1)
    }

    /// 8-neighbour trilinear SH lookup (ADR-041 §3). Missing cells
    /// contribute the fallback probe.
    pub fn sample(&self, world_pos: Vec3) -> ShL2 {
        let p = Vec3::new(
            world_pos.x / self.cell_size,
            world_pos.y / self.cell_size,
            world_pos.z / self.cell_size,
        );
        let cx = p.x.floor();
        let cy = p.y.floor();
        let cz = p.z.floor();
        let fx = p.x - cx;
        let fy = p.y - cy;
        let fz = p.z - cz;
        let base = (cx as i16, cy as i16, cz as i16);
        let mut accum = ShL2::ZERO;
        for n in 0..8u8 {
            let dx = (n & 1) as i16;
            let dy = ((n >> 1) & 1) as i16;
            let dz = ((n >> 2) & 1) as i16;
            let wx = if dx == 0 { 1.0 - fx } else { fx };
            let wy = if dy == 0 { 1.0 - fy } else { fy };
            let wz = if dz == 0 { 1.0 - fz } else { fz };
            let w = wx * wy * wz;
            let key = (base.0 + dx, base.1 + dy, base.2 + dz);
            let sh = self.probe_at(key).map(|p| p.sh).unwrap_or(self.fallback);
            accum = accum.add(&sh.scale(w));
        }
        accum
    }

    /// Convenience: trilinearly sample the probe set at `world_pos`
    /// and evaluate the cosine-convolved irradiance for normal `n`.
    pub fn evaluate_diffuse(&self, world_pos: Vec3, normal: Vec3) -> Vec3 {
        self.sample(world_pos).evaluate_irradiance(normal)
    }
}

/// Closed-form Ramamoorthi-Hanrahan reference irradiance for a
/// directional light of unit-white intensity along direction `d`
/// evaluated at surface normal `n`. Returns the scalar luminance
/// (apply to per-channel light colour by simple multiplication —
/// the directional-light SH expansion is linear in colour).
///
/// Formula derived from the addition theorem of spherical harmonics
/// (RH-2001 eq. 7-9):
///
/// ```text
/// E(n) / L = 1/4 + (n·d)/2 + (5/32) · (3 (n·d)² − 1)
/// ```
///
/// The truncated L2 series carries small negative leakage on the dark
/// side (~6% of peak); the engine clamps to ≥ 0 in
/// [`ShL2::evaluate_irradiance`]. This closed form is the unclamped
/// reference used by the convolution oracle test.
pub fn directional_light_irradiance_closed_form(d: Vec3, n: Vec3) -> f32 {
    let cos_theta = d.normalize_or_zero().dot(n.normalize_or_zero());
    0.25 + 0.5 * cos_theta + (5.0 / 32.0) * (3.0 * cos_theta * cos_theta - 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vec3(x: f32, y: f32, z: f32) -> Vec3 {
        Vec3::new(x, y, z)
    }

    #[test]
    fn dc_band_is_axis_independent() {
        // Y_00 is the constant 1/(2*sqrt(pi)) — direction-invariant.
        let basis_x = ShL2::basis(Vec3::X);
        let basis_z = ShL2::basis(Vec3::Z);
        assert!((basis_x[0] - basis_z[0]).abs() < 1e-6);
        assert!((basis_x[0] - SH_Y00).abs() < 1e-6);
    }

    #[test]
    fn directional_light_at_peak_matches_closed_form() {
        // L2 reconstruction of cosine lobe under unit white light.
        let d = vec3(0.0, 1.0, 0.0);
        let sh = ShL2::from_directional_light(d, Vec3::ONE);
        let e = sh.evaluate_irradiance(d);
        let ref_val = directional_light_irradiance_closed_form(d, d);
        // E(d, d) ≈ 1.0625 in unclamped form.
        assert!((ref_val - 1.0625).abs() < 1e-4);
        assert!((e.x - ref_val).abs() < 1e-4);
        assert!((e.y - ref_val).abs() < 1e-4);
        assert!((e.z - ref_val).abs() < 1e-4);
    }

    #[test]
    fn directional_light_back_side_carries_small_positive_leak() {
        // L2 truncation cannot reconstruct the cos-lobe's hard zero on
        // the dark side; the closed-form analytic value is ~0.0625 of
        // peak — a residual the rendering pipeline expects.
        let d = vec3(0.0, 1.0, 0.0);
        let sh = ShL2::from_directional_light(d, Vec3::ONE);
        let antipode = Vec3::new(0.0, -1.0, 0.0);
        let e = sh.evaluate_irradiance(antipode);
        let ref_val = directional_light_irradiance_closed_form(d, antipode);
        assert!((ref_val - 0.0625).abs() < 1e-4);
        assert!((e.x - ref_val).abs() < 1e-4);
        assert!((e.y - ref_val).abs() < 1e-4);
        assert!((e.z - ref_val).abs() < 1e-4);
    }

    #[test]
    fn directional_light_orthogonal_normal_matches_closed_form() {
        let d = vec3(0.0, 1.0, 0.0);
        let n = vec3(1.0, 0.0, 0.0);
        let sh = ShL2::from_directional_light(d, Vec3::ONE);
        let e = sh.evaluate_irradiance(n);
        let ref_val = directional_light_irradiance_closed_form(d, n);
        // At cos_theta = 0: 0.25 - 5/32 = 0.09375
        assert!((ref_val - 0.09375).abs() < 1e-4);
        assert!((e.x - ref_val).abs() < 1e-4);
    }

    #[test]
    fn ambient_probe_evaluates_to_input_radiance() {
        // A constant sky should reconstruct to E(n) ≈ radiance · π / Y_00² · A_0?
        // For from_ambient(L) the DC coefficient is L / Y_00. Evaluating
        // gives Y_00 * A_0 * (L / Y_00) = A_0 * L = π · L on the DC term.
        let l = vec3(0.5, 0.7, 0.9);
        let sh = ShL2::from_ambient(l);
        let e = sh.evaluate_irradiance(Vec3::Y);
        let expected = vec3(PI * l.x, PI * l.y, PI * l.z);
        assert!((e.x - expected.x).abs() < 1e-4);
        assert!((e.y - expected.y).abs() < 1e-4);
        assert!((e.z - expected.z).abs() < 1e-4);
    }

    #[test]
    fn closed_form_swept_matches_evaluate() {
        // Across 12 sample directions on the equator, the unclamped SH
        // evaluation must match the analytic closed form to 1e-4.
        let light = vec3(0.0, 1.0, 0.0);
        let sh = ShL2::from_directional_light(light, Vec3::ONE);
        for k in 0..12 {
            let theta = (k as f32) * (PI / 6.0);
            // Normals tilted above horizon to stay on the lit hemisphere
            // where the unclamped analytic == clamped numeric.
            let n = Vec3::new(theta.cos() * 0.5, 0.8, theta.sin() * 0.5).normalize_or_zero();
            let e = sh.evaluate_irradiance(n);
            let r = directional_light_irradiance_closed_form(light, n);
            assert!(r > 0.0);
            assert!(
                (e.x - r).abs() < 1e-4,
                "k={k}: numeric {e:?} vs analytic {r}"
            );
        }
    }

    #[test]
    fn trilinear_sample_at_probe_returns_that_probe() {
        let mut set = IblProbeSet::new(2.0, ShL2::ZERO);
        let sh = ShL2::from_directional_light(Vec3::Y, Vec3::new(1.0, 0.5, 0.25));
        assert!(set.insert(Probe {
            position: vec3(0.0, 0.0, 0.0),
            sh,
        }));
        // Trilinear lookup exactly at the cell origin returns the
        // single neighbour with weight 1.0; missing neighbours all
        // contribute fallback=ZERO with their respective weights but
        // since the centre weight is the only non-zero one when fx,
        // fy, fz are all 0, the result equals the probe SH exactly.
        let s = set.sample(vec3(0.0, 0.0, 0.0));
        for i in 0..9 {
            assert!((s.coeffs[i].x - sh.coeffs[i].x).abs() < 1e-6);
            assert!((s.coeffs[i].y - sh.coeffs[i].y).abs() < 1e-6);
        }
    }

    #[test]
    fn trilinear_sample_midway_between_two_probes() {
        let mut set = IblProbeSet::new(2.0, ShL2::ZERO);
        let sh0 = ShL2::from_directional_light(Vec3::Y, vec3(1.0, 0.0, 0.0));
        let sh1 = ShL2::from_directional_light(Vec3::Y, vec3(0.0, 0.0, 1.0));
        set.insert(Probe {
            position: vec3(0.0, 0.0, 0.0),
            sh: sh0,
        });
        set.insert(Probe {
            position: vec3(2.0, 0.0, 0.0),
            sh: sh1,
        });
        // Midway along x at cell-boundary fraction 0.5.
        let mid = set.sample(vec3(1.0, 0.0, 0.0));
        // For each band the result should be (sh0 + sh1) * 0.5.
        for i in 0..9 {
            let expect_x = (sh0.coeffs[i].x + sh1.coeffs[i].x) * 0.5;
            let expect_z = (sh0.coeffs[i].z + sh1.coeffs[i].z) * 0.5;
            assert!(
                (mid.coeffs[i].x - expect_x).abs() < 1e-5,
                "band {i} x: got {} want {expect_x}",
                mid.coeffs[i].x
            );
            assert!(
                (mid.coeffs[i].z - expect_z).abs() < 1e-5,
                "band {i} z: got {} want {expect_z}",
                mid.coeffs[i].z
            );
        }
    }

    #[test]
    fn insertion_respects_max_probes_cap() {
        let mut set = IblProbeSet::new(1.0, ShL2::ZERO);
        for i in 0..MAX_PROBES {
            let x = i as f32;
            assert!(set.insert(Probe {
                position: vec3(x, 0.0, 0.0),
                sh: ShL2::ZERO,
            }));
        }
        assert_eq!(set.len(), MAX_PROBES);
        // 129th probe in a new cell is rejected.
        assert!(!set.insert(Probe {
            position: vec3(MAX_PROBES as f32, 0.0, 0.0),
            sh: ShL2::ZERO,
        }));
        // But updating an existing probe (same cell) is fine.
        assert!(set.insert(Probe {
            position: vec3(0.0, 0.0, 0.0),
            sh: ShL2::from_ambient(Vec3::ONE),
        }));
        assert_eq!(set.len(), MAX_PROBES);
    }

    #[test]
    fn fallback_is_used_when_grid_is_empty() {
        let fallback = ShL2::from_ambient(vec3(0.1, 0.1, 0.1));
        let set = IblProbeSet::new(2.0, fallback);
        let e = set.evaluate_diffuse(vec3(0.0, 0.0, 0.0), Vec3::Y);
        // Pure ambient → cosine-convolved DC band == π · radiance.
        assert!((e.x - PI * 0.1).abs() < 1e-4);
        assert!((e.y - PI * 0.1).abs() < 1e-4);
        assert!((e.z - PI * 0.1).abs() < 1e-4);
    }

    #[test]
    fn entries_remain_sorted_after_random_inserts() {
        let mut set = IblProbeSet::new(1.0, ShL2::ZERO);
        // Insert in non-sorted order; expect the entries Vec to come
        // out sorted by key.
        let positions = [
            vec3(3.5, 0.0, 0.0),
            vec3(-2.0, 1.5, 0.0),
            vec3(0.5, 0.0, 4.0),
            vec3(0.5, 0.0, -1.0),
            vec3(0.0, 0.0, 0.0),
        ];
        for p in positions {
            set.insert(Probe {
                position: p,
                sh: ShL2::ZERO,
            });
        }
        for w in set.entries().windows(2) {
            assert!(w[0].0 <= w[1].0);
        }
        assert_eq!(set.len(), 5);
    }
}
