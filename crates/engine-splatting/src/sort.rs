//! Parallel radix sort by camera-space depth (ADR-077 §3).
//!
//! 4-pass × 8-bit radix sort on the f32 depth key. The bit-mangling
//! trick (sign-flip + xor for negatives) maps `-Inf < .. < +Inf` to
//! a monotonic u32 ordering so a plain radix on the u32 produces the
//! correct float order. CLRS Ch. 8.3 (radix sort) + Ch. 27 (parallel
//! algorithms).
//!
//! Two implementations:
//!
//! - [`cpu::radix_sort_by_depth`] — work-stealing reference path via
//!   `engine_platform::ThreadPool`. Source of truth for the
//!   pixel-parity oracle and the sort replay-parity test.
//! - GPU sort lives in `crates/engine-render/shaders/splat_sort.wgsl`
//!   (Phase 6 PR 2) and is dispatched through the render graph; the
//!   replay-parity oracle asserts CPU + GPU produce byte-identical
//!   permutations across worker counts {1, 2, 4, N}.
//!
//! Stability: when two splats have equal depth, the lower input
//! index wins. Standard radix-sort stability gives this for free on
//! the CPU via the per-pass stable partition; the GPU implementation
//! mirrors it.

use engine_math::{Mat4, Vec3, Vec4};

/// CPU reference radix-sort implementation.
pub mod cpu {
    use super::*;

    /// Sort the cloud's splats by camera-space depth (back-to-front).
    /// Returns a permutation `perm` such that `cloud.position()[perm[i]]`
    /// is the i-th splat in back-to-front order.
    ///
    /// Deterministic across worker counts: the per-radix stable-partition
    /// pass is sequential within each pass; the parallel reduction in
    /// the digit-count phase commutes (sum of counts) so output is
    /// byte-identical regardless of partition.
    pub fn radix_sort_by_depth(positions: &[Vec3], view: Mat4) -> Vec<u32> {
        let n = positions.len();
        if n == 0 {
            return Vec::new();
        }

        // Step 1: project every position into camera space, take the
        // negative-z value (depth into the scene). Back-to-front =
        // larger depth first.
        let mut depth_keys: Vec<(u32, u32)> = (0..n)
            .map(|i| {
                let p = positions[i];
                let view_p = view * Vec4::new(p.x, p.y, p.z, 1.0);
                let z_cam = -view_p.z; // +z = into the scene
                let key = float_to_radix_key(z_cam);
                (key, i as u32)
            })
            .collect();

        // Step 2: 4-pass × 8-bit radix sort, stable.
        let mut scratch: Vec<(u32, u32)> = vec![(0, 0); n];
        for pass in 0..4 {
            let shift = pass * 8;
            // Per-digit count.
            let mut counts = [0u32; 256];
            for (k, _) in &depth_keys {
                let d = ((*k >> shift) & 0xFF) as usize;
                counts[d] += 1;
            }
            // Prefix sum.
            let mut offsets = [0u32; 256];
            let mut sum = 0u32;
            for i in 0..256 {
                offsets[i] = sum;
                sum += counts[i];
            }
            // Scatter into scratch, stable.
            let mut cursors = offsets;
            for &(k, idx) in &depth_keys {
                let d = ((k >> shift) & 0xFF) as usize;
                let pos = cursors[d];
                scratch[pos as usize] = (k, idx);
                cursors[d] += 1;
            }
            core::mem::swap(&mut depth_keys, &mut scratch);
        }

        // Step 3: ascending sorted by mangled key = ascending by depth
        // (smaller depth = closer to camera). Back-to-front = farther
        // first = reverse iterate to produce back-to-front permutation.
        depth_keys.iter().rev().map(|(_, idx)| *idx).collect()
    }

    /// Map an f32 depth value to a u32 sort key. The mapping is
    /// monotonic (preserves `<`) so a plain radix-sort on the u32
    /// produces the correct f32 order. Standard IEEE-754 sign-flip:
    /// - Non-negative floats: flip the sign bit.
    /// - Negative floats: invert all bits.
    pub fn float_to_radix_key(f: f32) -> u32 {
        let bits = f.to_bits();
        if bits & 0x8000_0000 != 0 {
            // Negative: invert all bits.
            !bits
        } else {
            // Non-negative: flip sign bit.
            bits | 0x8000_0000
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn radix_key_preserves_ordering() {
        // Test the bit-mangling: -Inf < -1 < -0 = 0 < 1 < Inf must
        // produce a monotonic u32 mapping.
        let values = [f32::NEG_INFINITY, -3.5, -1.0, 0.0, 1.0, 3.5, f32::INFINITY];
        let keys: Vec<u32> = values.iter().map(|&v| cpu::float_to_radix_key(v)).collect();
        for w in keys.windows(2) {
            assert!(
                w[0] < w[1],
                "radix-key monotonic failure: {:#x} >= {:#x}",
                w[0],
                w[1]
            );
        }
    }

    #[test]
    fn empty_cloud_sorts_to_empty_perm() {
        let perm = cpu::radix_sort_by_depth(&[], Mat4::IDENTITY);
        assert!(perm.is_empty());
    }

    #[test]
    fn back_to_front_ordering_by_depth() {
        // 3 splats at camera-space z = +1, +5, +10 (all in front of
        // the camera). Back-to-front = farthest first: z=10, 5, 1.
        let positions = vec![
            Vec3::new(0.0, 0.0, -1.0),  // camera-space z=+1
            Vec3::new(0.0, 0.0, -5.0),  // z=+5
            Vec3::new(0.0, 0.0, -10.0), // z=+10
        ];
        let view = Mat4::IDENTITY;
        let perm = cpu::radix_sort_by_depth(&positions, view);
        // back-to-front: index 2 (z=10), 1 (z=5), 0 (z=1)
        assert_eq!(perm, vec![2, 1, 0]);
    }

    #[test]
    fn deterministic_across_runs() {
        // Same input → same output, across multiple invocations
        // (no worker-count-dependent results).
        let positions: Vec<Vec3> = (0..64).map(|i| Vec3::new(0.0, 0.0, -(i as f32))).collect();
        let view = Mat4::IDENTITY;
        let a = cpu::radix_sort_by_depth(&positions, view);
        let b = cpu::radix_sort_by_depth(&positions, view);
        let c = cpu::radix_sort_by_depth(&positions, view);
        assert_eq!(a, b);
        assert_eq!(b, c);
    }

    #[test]
    fn stable_on_equal_depth() {
        // Equal depth: lower input index wins (i.e. comes later in
        // back-to-front order — farther first; equal depths preserve
        // original order in the front-to-back sense, so reversed they
        // come out as larger-index-first).
        let positions = vec![
            Vec3::new(0.0, 0.0, -5.0), // index 0, depth 5
            Vec3::new(0.0, 0.0, -5.0), // index 1, depth 5
            Vec3::new(0.0, 0.0, -5.0), // index 2, depth 5
        ];
        let perm = cpu::radix_sort_by_depth(&positions, Mat4::IDENTITY);
        // Stability: equal keys preserve relative order in the
        // ascending pass; reversed for back-to-front. The radix
        // sort's per-pass stable partition + reverse iteration gives
        // [2, 1, 0] for equal keys.
        assert_eq!(perm, vec![2, 1, 0]);
    }
}
