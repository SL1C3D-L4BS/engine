//! Phase 6 PR 1a (ADR-083 §5) — UpscalePass end-to-end dispatch.
//!
//! Drives [`UpscalePass`] through the ADR-075 §1 six-step template +
//! the ADR-083 §2 registry short-circuit against a real adapter:
//! allocate src/dst Rgba16Float textures + a linear sampler, register
//! them in a [`TransientResourceTable`], compile a single-pass graph,
//! attach an [`UpscalerRegistry`] via the new
//! [`PassContext::upscaler`] slot, execute, verify no panic.
//!
//! The test ships in two variants — one per cascade slot we wired in
//! tree (OwnedBilinear + VendorFsr/EASU). Both run through the same
//! harness; the only difference is the registry the runner attaches.
//! Skips gracefully on hosts without a Vulkan loader.

use std::panic::{self, AssertUnwindSafe};

use engine_gpu::{
    CommandEncoder, Device, DeviceLimits, Extent3d, Sampler, SamplerDesc, Texture, TextureDesc,
    TextureDimension, TextureUsage,
};
use engine_render::{
    GpuFrameContext, RenderGraph, ResourceId, TransientResourceTable, UpscalePass,
    UpscalerRegistry, contracts,
};

fn try_device() -> Option<Device> {
    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        Device::new(DeviceLimits::Tier1Minimum, false)
    }));
    match result {
        Ok(Ok(d)) => Some(d),
        Ok(Err(e)) => {
            eprintln!("[upscale_dispatch] no compatible adapter (skipping): {e}");
            None
        }
        Err(_) => {
            eprintln!("[upscale_dispatch] wgpu has no backend enabled (skipping)");
            None
        }
    }
}

/// Build the 3 resources [`UpscalePass`] needs (src view + sampler +
/// dst storage view) at the given internal / display extents.
fn build_upscale_pool(
    device: &Device,
    src_extent: (u32, u32),
    dst_extent: (u32, u32),
) -> TransientResourceTable {
    let mut table = TransientResourceTable::new();
    let src = Texture::new(
        device,
        &TextureDesc {
            label: "upscale.src",
            extent: Extent3d::new_2d(src_extent.0, src_extent.1),
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: contracts::LIT_COLOR_FORMAT,
            usage: TextureUsage::TEXTURE_BINDING | TextureUsage::COPY_DST,
        },
    );
    let dst = Texture::new(
        device,
        &TextureDesc {
            label: "upscale.dst",
            extent: Extent3d::new_2d(dst_extent.0, dst_extent.1),
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: contracts::LIT_COLOR_FORMAT,
            usage: TextureUsage::STORAGE_BINDING | TextureUsage::COPY_SRC,
        },
    );
    let sampler = Sampler::new(device, SamplerDesc::linear_repeat());
    table.register_texture(ResourceId(0), src);
    table.register_texture(ResourceId(1), dst);
    table.register_sampler(ResourceId(2), sampler);
    table
}

/// Helper: stand up a one-pass graph with [`UpscalePass`], install both
/// shipped pipelines, compile, execute with the supplied registry.
fn run_one_frame(device: &Device, registry: &UpscalerRegistry) {
    let pool = build_upscale_pool(device, (16, 16), (32, 32));
    let mut graph = RenderGraph::new();
    graph.add_pass(UpscalePass::new(
        ResourceId(0), // resolved input
        ResourceId(1), // upscaled output
        ResourceId(2), // linear sampler
    ));
    graph
        .install_pipelines(device)
        .expect("UpscalePass pipelines install");
    graph.compile().expect("graph compiles");
    let mut user: () = ();
    let mut encoder = CommandEncoder::new(device, "upscale.encoder");
    let gpu = GpuFrameContext {
        device,
        encoder: &mut encoder,
    };
    graph
        .execute(0, &mut user, Some(gpu), Some(&pool), Some(registry))
        .expect("execute completes");
    let _token = device.queue().submit(encoder);
}

/// The bilinear pipeline runs end-to-end when the cascade selects it.
/// The forced-bilinear registry pins the choice to [`OwnedBilinear`].
#[test]
fn upscale_pass_dispatches_bilinear() {
    let Some(device) = try_device() else {
        return;
    };
    let cfg = engine_render::UpscalerConfig {
        provider: engine_render::Provider::OwnedBilinear,
        quality: engine_render::Quality::Balanced,
    };
    let registry = UpscalerRegistry::with_phase6_defaults_from_config(&cfg);
    run_one_frame(&device, &registry);
}

/// The FSR-EASU pipeline runs end-to-end when the cascade selects FSR.
/// The forced-FSR registry pins the choice to [`VendorFsr`] (ADR-076
/// step 2 closure: `VendorFsr::supports() == true` on every adapter
/// the engine can reach).
#[test]
fn upscale_pass_dispatches_fsr_easu() {
    let Some(device) = try_device() else {
        return;
    };
    let cfg = engine_render::UpscalerConfig {
        provider: engine_render::Provider::Fsr,
        quality: engine_render::Quality::Balanced,
    };
    let registry = UpscalerRegistry::with_phase6_defaults_from_config(&cfg);
    run_one_frame(&device, &registry);
}

/// The default Auto cascade picks FSR on every adapter (DLSS + XeSS
/// decline at supports(); FSR is the next in line). End-to-end
/// dispatch must complete on the same harness.
#[test]
fn upscale_pass_dispatches_auto_cascade() {
    let Some(device) = try_device() else {
        return;
    };
    let registry = UpscalerRegistry::with_phase6_defaults();
    run_one_frame(&device, &registry);
}

/// Without an upscaler registry attached, [`UpscalePass::record`]
/// short-circuits cleanly — the no-upscale-variant graph shape from
/// ADR-053. Verifies the new `ctx.upscaler` short-circuit doesn't
/// regress the no-upscale path.
#[test]
fn upscale_pass_no_registry_short_circuits() {
    let Some(device) = try_device() else {
        return;
    };
    let pool = build_upscale_pool(&device, (16, 16), (32, 32));
    let mut graph = RenderGraph::new();
    graph.add_pass(UpscalePass::new(
        ResourceId(0),
        ResourceId(1),
        ResourceId(2),
    ));
    graph
        .install_pipelines(&device)
        .expect("UpscalePass pipelines install");
    graph.compile().expect("graph compiles");
    let mut user: () = ();
    let mut encoder = CommandEncoder::new(&device, "upscale.no_registry.encoder");
    let gpu = GpuFrameContext {
        device: &device,
        encoder: &mut encoder,
    };
    graph
        .execute(0, &mut user, Some(gpu), Some(&pool), None)
        .expect("execute completes (no-op path)");
    let _token = device.queue().submit(encoder);
}
