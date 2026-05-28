// SplatSort — parallel radix sort by camera-space depth (ADR-077 §3).
//
// 4-pass × 8-bit radix on the f32 depth key. Polaris-compatible: no
// subgroup intrinsics, no f16, only standard 32-bit atomicAdd on
// global memory. Workgroup (256, 1, 1).
//
// The CPU oracle reference is `engine_splatting::sort::cpu::radix_sort_by_depth`.
// Both implementations must produce byte-identical permutations
// across worker counts {1, 2, 4, N} — the replay-parity oracle in
// `crates/engine-splatting/tests/sort_replay_parity.rs` (Phase 6 PR
// 2 follow-up) asserts this.

struct SortPushConstants {
    view : mat4x4<f32>,
    splat_count : u32,
    radix_pass : u32,        // 0, 1, 2, 3
    reserved : vec2<u32>,
};

@group(0) @binding(0) var<storage, read>       positions : array<vec3<f32>>;
@group(0) @binding(1) var<storage, read_write> in_keys    : array<u32>;
@group(0) @binding(2) var<storage, read_write> in_indices : array<u32>;
@group(0) @binding(3) var<storage, read_write> out_keys    : array<u32>;
@group(0) @binding(4) var<storage, read_write> out_indices : array<u32>;
@group(0) @binding(5) var<storage, read_write> bin_counts  : array<atomic<u32>>; // 256
@group(1) @binding(0) var<uniform> push : SortPushConstants;

// IEEE-754 sign-flip: maps f32 ordering onto u32 ordering monotonically.
fn float_to_radix_key(f : f32) -> u32 {
    let bits = bitcast<u32>(f);
    if ((bits & 0x80000000u) != 0u) {
        return ~bits;
    }
    return bits | 0x80000000u;
}

// cs_init: per-splat, project the position into camera space, take
// the negative-z value (depth into the scene), pack as a radix key.
@compute
@workgroup_size(256, 1, 1)
fn cs_init(@builtin(global_invocation_id) gid : vec3<u32>) {
    let i = gid.x;
    if (i >= push.splat_count) { return; }
    let p = positions[i];
    let view_p = push.view * vec4<f32>(p, 1.0);
    let z_cam = -view_p.z;
    in_keys[i] = float_to_radix_key(z_cam);
    in_indices[i] = i;
}

// cs_count: per-splat, atomically increment the bin for this pass's
// 8-bit digit. The 256-bin counts feed the prefix-sum + scatter that
// the host-side driver invokes after this dispatch returns.
@compute
@workgroup_size(256, 1, 1)
fn cs_count(@builtin(global_invocation_id) gid : vec3<u32>) {
    let i = gid.x;
    if (i >= push.splat_count) { return; }
    let shift = push.radix_pass * 8u;
    let digit = (in_keys[i] >> shift) & 0xFFu;
    atomicAdd(&bin_counts[digit], 1u);
}

// cs_scatter: per-splat, write the (key, index) pair into the
// scatter location determined by the prefix-sum'd bin counts. The
// host driver builds `bin_counts` as the exclusive-prefix offsets
// before this dispatch; this kernel performs the stable partition.
//
// Note: this kernel is *not* atomic-correctness-on-its-own — it
// assumes the host has reset `bin_counts` to the exclusive-prefix
// offsets, and that the per-bin offset increments here are serial
// per bin (which works because each pass's stable partition is
// sequential within a bin). The host-side wrapper enforces this
// by dispatching one workgroup per pass.
@compute
@workgroup_size(256, 1, 1)
fn cs_scatter(@builtin(global_invocation_id) gid : vec3<u32>) {
    let i = gid.x;
    if (i >= push.splat_count) { return; }
    let shift = push.radix_pass * 8u;
    let digit = (in_keys[i] >> shift) & 0xFFu;
    let pos = atomicAdd(&bin_counts[digit], 1u);
    out_keys[pos] = in_keys[i];
    out_indices[pos] = in_indices[i];
}
