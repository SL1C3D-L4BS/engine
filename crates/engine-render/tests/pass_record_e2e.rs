//! Phase 5.5 A.2b-ii (cont) end-to-end pass-record smoke.
//!
//! Drives [`CullPass`] through the full ADR-075 §1 six-step template
//! against a real adapter: register 5 buffers in a
//! [`TransientResourceTable`], compile a single-pass graph, execute,
//! verify no panic. The proof that the foundation surface
//! (`ResourceResolver` + `PassContext.resources` +
//! `RenderGraph::execute(_, _, _, resources)`) lets a pass's
//! `record()` body construct a bind group against the auto-derived
//! layout, dispatch the kernel, and complete the encoder submission.
//!
//! Skips gracefully on hosts without a Vulkan loader.

use std::panic::{self, AssertUnwindSafe};

use engine_gpu::{
    Buffer, BufferDesc, BufferUsage, CommandEncoder, Device, DeviceLimits, Extent3d, SamplerDesc,
    Texture, TextureDesc, TextureDimension, TextureFormat, TextureUsage,
};
use engine_render::{
    CullPass, GpuFrameContext, LightingAccumulationPass, RenderGraph, ResourceId, ResourceResolver,
    TransientResourceTable, contracts,
};

fn try_device() -> Option<Device> {
    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        Device::new(DeviceLimits::Tier1Minimum, false)
    }));
    match result {
        Ok(Ok(d)) => Some(d),
        Ok(Err(e)) => {
            eprintln!("[pass_record_e2e] no compatible adapter (skipping): {e}");
            None
        }
        Err(_) => {
            eprintln!("[pass_record_e2e] wgpu has no backend enabled (skipping)");
            None
        }
    }
}

/// Build the 5 buffers a CullPass needs and register them under the
/// canonical [`ResourceId`]s.
fn build_cull_pool(device: &Device) -> TransientResourceTable {
    let mut table = TransientResourceTable::new();
    // 16-byte frustum UBO placeholder (the real layout is 6 × vec4 =
    // 96 bytes, but for the smoke the size only matters because the
    // shader expects it sampled, not zero-bytes).
    let frustum = Buffer::new(
        device,
        &BufferDesc {
            label: "smoke.frustum",
            // Aligned to 16; 6 × 16-byte vec4 = 96.
            size: 96,
            usage: BufferUsage::UNIFORM | BufferUsage::COPY_DST,
        },
    );
    // 1 instance × 48-byte InstanceEntry = 48 bytes.
    let instances = Buffer::new(
        device,
        &BufferDesc {
            label: "smoke.instances",
            size: 48,
            usage: BufferUsage::STORAGE | BufferUsage::COPY_DST,
        },
    );
    // 1 mesh × 16-byte MeshEntry = 16 bytes.
    let meshes = Buffer::new(
        device,
        &BufferDesc {
            label: "smoke.meshes",
            size: 16,
            usage: BufferUsage::STORAGE | BufferUsage::COPY_DST,
        },
    );
    // 1 draw × 20-byte DrawIndirect = 20 bytes (round up to 32 for
    // alignment safety).
    let draws = Buffer::new(
        device,
        &BufferDesc {
            label: "smoke.draws",
            size: 32,
            usage: BufferUsage::STORAGE | BufferUsage::COPY_DST,
        },
    );
    // 1 atomic u32 = 4 bytes (storage minimum is 16 in practice).
    let draw_count = Buffer::new(
        device,
        &BufferDesc {
            label: "smoke.draw_count",
            size: 16,
            usage: BufferUsage::STORAGE | BufferUsage::COPY_DST,
        },
    );
    table.register_buffer(ResourceId(0), frustum);
    table.register_buffer(ResourceId(1), instances);
    table.register_buffer(ResourceId(2), meshes);
    table.register_buffer(ResourceId(3), draws);
    table.register_buffer(ResourceId(4), draw_count);
    table
}

/// CullPass executes against a real device + the foundation resolver.
/// Proves the 6-step template (Step 1 gpu / Step 2 pipeline / Step 3
/// resources / Step 4 resolve / Step 5 begin+set+dispatch / Step 6
/// end-of-scope) completes the encoder submission without panic.
#[test]
fn cull_pass_executes_via_resolver() {
    let Some(device) = try_device() else {
        return;
    };
    let queue = device.queue();
    let pool = build_cull_pool(&device);
    let resolver: &dyn ResourceResolver = &pool;

    // Construct + install pipeline + add to graph.
    let mut graph = RenderGraph::new();
    graph.add_pass(CullPass::new(
        ResourceId(1), // render_queue (instances) — graph-flow read.
        ResourceId(3), // indirect_draws — graph-flow write.
        ResourceId(0), // frustum_uniforms.
        ResourceId(2), // meshes.
        ResourceId(4), // draw_count.
    ));
    graph
        .install_pipelines(&device)
        .expect("CullPass pipeline installs");
    graph.compile().expect("graph compiles");

    // Execute through the resolver path. user is a unit scratchpad.
    let mut user: () = ();
    let mut encoder = CommandEncoder::new(&device, "smoke.encoder");
    let gpu = GpuFrameContext {
        device: &device,
        encoder: &mut encoder,
    };
    graph
        .execute(0, &mut user, Some(gpu), Some(resolver))
        .expect("execute completes");
    let _token = queue.submit(encoder);
}

/// Helper: allocate a single-pixel 2D color texture (full HDR target
/// stand-in for smoke purposes). 1×1 keeps the bind-group + render-
/// pass valid without consuming real GPU memory.
fn tiny_color(device: &Device, label: &str, format: TextureFormat) -> Texture {
    Texture::new(
        device,
        &TextureDesc {
            label,
            extent: Extent3d::new_2d(1, 1),
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format,
            usage: TextureUsage::RENDER_ATTACHMENT
                | TextureUsage::TEXTURE_BINDING
                | TextureUsage::COPY_DST,
        },
    )
}

/// Helper: 1×1 depth texture.
fn tiny_depth(device: &Device, label: &str) -> Texture {
    Texture::new(
        device,
        &TextureDesc {
            label,
            extent: Extent3d::new_2d(1, 1),
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Depth32Float,
            usage: TextureUsage::RENDER_ATTACHMENT | TextureUsage::TEXTURE_BINDING,
        },
    )
}

/// LightingAccumulationPass executes against a real device through the
/// resolver path. Exercises the most binding-heavy of the wired passes:
/// 3 bind groups (frame UBO, cluster + 3 SSBOs, 5 textures + 1
/// sampler_comparison) + a full-screen draw against a single
/// `lit_color` color attachment. Proves the ADR-075 §1 template scales
/// to the lighting pass shape end-to-end.
#[test]
fn lighting_accumulation_executes_via_resolver() {
    let Some(device) = try_device() else {
        return;
    };
    let queue = device.queue();
    let mut pool = TransientResourceTable::new();

    // 3 G-buffer color textures (matching the contracts MRT formats),
    // 1 depth, 1 shadow atlas (depth-only).
    pool.register_texture(
        ResourceId(10),
        tiny_color(
            &device,
            "smoke.gbuf_ar",
            contracts::GBUFFER_ALBEDO_ROUGHNESS_FORMAT,
        ),
    );
    pool.register_texture(
        ResourceId(11),
        tiny_color(
            &device,
            "smoke.gbuf_nm",
            contracts::GBUFFER_NORMAL_METALLIC_FORMAT,
        ),
    );
    pool.register_texture(
        ResourceId(12),
        tiny_color(
            &device,
            "smoke.gbuf_md",
            contracts::GBUFFER_MOTION_DEPTH_FORMAT,
        ),
    );
    pool.register_texture(ResourceId(13), tiny_depth(&device, "smoke.depth"));
    pool.register_texture(ResourceId(14), tiny_depth(&device, "smoke.shadow"));
    pool.register_texture(
        ResourceId(15),
        tiny_color(&device, "smoke.lit", contracts::LIT_COLOR_FORMAT),
    );

    // Buffers: frame UBO + cluster UBO + lights + cells + light_indices.
    // Sizes match the WGSL struct layouts.
    pool.register_buffer(
        ResourceId(20),
        Buffer::new(
            &device,
            &BufferDesc {
                label: "smoke.frame_ubo",
                // FullScreenUniforms: mat4x4 + vec4 + vec2 + vec2 = 96B.
                size: 96,
                usage: BufferUsage::UNIFORM | BufferUsage::COPY_DST,
            },
        ),
    );
    pool.register_buffer(
        ResourceId(21),
        Buffer::new(
            &device,
            &BufferDesc {
                label: "smoke.cluster_ubo",
                // ClusterUniforms: mat4x4 + u32 + uvec3 + 2 × f32 + vec2 = 112B.
                size: 112,
                usage: BufferUsage::UNIFORM | BufferUsage::COPY_DST,
            },
        ),
    );
    pool.register_buffer(
        ResourceId(22),
        Buffer::new(
            &device,
            &BufferDesc {
                label: "smoke.lights",
                size: 64,
                usage: BufferUsage::STORAGE | BufferUsage::COPY_DST,
            },
        ),
    );
    pool.register_buffer(
        ResourceId(23),
        Buffer::new(
            &device,
            &BufferDesc {
                label: "smoke.cells",
                size: 32,
                usage: BufferUsage::STORAGE | BufferUsage::COPY_DST,
            },
        ),
    );
    pool.register_buffer(
        ResourceId(24),
        Buffer::new(
            &device,
            &BufferDesc {
                label: "smoke.light_indices",
                size: 32,
                usage: BufferUsage::STORAGE | BufferUsage::COPY_DST,
            },
        ),
    );

    // Shadow comparison sampler (reverse-Z PCF).
    pool.register_sampler(
        ResourceId(30),
        engine_gpu::Sampler::new(&device, SamplerDesc::shadow_pcf()),
    );

    let resolver: &dyn ResourceResolver = &pool;

    let mut graph = RenderGraph::new();
    graph.add_pass(LightingAccumulationPass::new(
        ResourceId(10), // gbuf_ar
        ResourceId(11), // gbuf_nm
        ResourceId(12), // gbuf_md
        ResourceId(13), // depth
        ResourceId(23), // cluster_cells
        ResourceId(22), // lights
        ResourceId(14), // shadow_atlas
        ResourceId(15), // lit_color (output)
        ResourceId(20), // frame_uniforms
        ResourceId(21), // cluster_uniforms
        ResourceId(24), // light_indices
        ResourceId(30), // shadow_sampler
    ));
    graph
        .install_pipelines(&device)
        .expect("LightingAccumulationPass pipeline installs");
    graph.compile().expect("graph compiles");

    let mut user: () = ();
    let mut encoder = CommandEncoder::new(&device, "smoke.lighting.encoder");
    let gpu = GpuFrameContext {
        device: &device,
        encoder: &mut encoder,
    };
    graph
        .execute(0, &mut user, Some(gpu), Some(resolver))
        .expect("execute completes");
    let _token = queue.submit(encoder);
}
