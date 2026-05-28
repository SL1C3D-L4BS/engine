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

use engine_gpu::{Buffer, BufferDesc, BufferUsage, CommandEncoder, Device, DeviceLimits};
use engine_render::{
    CullPass, GpuFrameContext, RenderGraph, ResourceId, ResourceResolver, TransientResourceTable,
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
