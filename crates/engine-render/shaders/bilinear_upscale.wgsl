// BilinearUpscalePass — owned-fallback GPU 2× bilinear upsampler
// (ADR-005, ADR-083 §4).
//
// Workgroup (8, 8, 1). Reads the TAA-resolved HDR target at internal
// resolution; writes the upscaled HDR target at display resolution
// via a `textureSampleLevel` against a linear sampler. The pixel
// math matches `engine_raster::upscale::bilinear_upscale` (the CPU
// oracle) so the new GPU dispatch and the historical CPU reference
// produce byte-comparable output up to RADV's f32 rounding.
//
// This shader replaces the Phase 5.5 documented "CPU oracle
// delegation" path. `OwnedBilinear::upscale()` now dispatches this
// compute pipeline directly so the cascade's final fallback is
// itself GPU-resident.

@group(0) @binding(0) var src : texture_2d<f32>;
@group(0) @binding(1) var src_sampler : sampler;
@group(0) @binding(2) var dst : texture_storage_2d<rgba16float, write>;

@compute
@workgroup_size(8, 8, 1)
fn cs_main(@builtin(global_invocation_id) gid : vec3<u32>) {
    let dst_extent = vec2<u32>(textureDimensions(dst).xy);
    if (gid.x >= dst_extent.x || gid.y >= dst_extent.y) {
        return;
    }
    // Sample at the destination pixel's centre, mapped to source UV
    // by the ratio of extents. textureSampleLevel with a linear
    // sampler performs the 2D bilinear interpolation in hardware.
    let uv = (vec2<f32>(f32(gid.x), f32(gid.y)) + vec2<f32>(0.5))
        / vec2<f32>(f32(dst_extent.x), f32(dst_extent.y));
    let sampled = textureSampleLevel(src, src_sampler, uv, 0.0);
    textureStore(dst, vec2<i32>(i32(gid.x), i32(gid.y)), sampled);
}
