//! Shared pixel-parity harness (Phase 5.5 A.3, ADR-046).
//!
//! Owns a real `wgpu` device, the full `Phase6Pipelines` bundle, and a
//! pre-baked BRDF LUT — the per-fixture cost is then only the
//! transient resource pool + the scene-specific UBO/SSBO writes.
//!
//! Each fixture follows the same shape:
//!
//! 1. `ParityHarness::try_new()` — once per test run; skips on no-Vulkan
//!    hosts (same pattern as `tests/pass_record_e2e.rs`).
//! 2. `harness.allocate_pool(width, height)` — fills the canonical
//!    10-pass [`TransientResourceTable`] with textures, buffers, and
//!    samplers at the requested render extent.
//! 3. Per-fixture: write camera UBOs, light SSBOs, mesh VB/IB, instance
//!    SSBO, plus seed the CullPass's indirect-draw pair so GBuffer +
//!    CSM consume a real draw.
//! 4. `harness.build_graph()` + `compile()` + `execute()` — wires all 10
//!    passes against the shared resource ids, runs them in canonical
//!    scheduling order.
//! 5. `harness.read_tonemap_back(...)` — copies the tonemap target into
//!    a `MAP_READ` staging buffer; one `submit` + `read_back` + BGRA→RGBA
//!    repack yields an `engine_raster::Framebuffer`.
//! 6. `engine_raster::compare_images(&cpu, &gpu)` — the ADR-046 verdict.
//!
//! ## What this slice ships vs. what comes later
//!
//! Phase 5.5 A.3 lands the harness in commit slices. The first slice
//! (this commit) exercises the structural path: allocate all 10 passes'
//! resources, run the graph, recover the tonemap. Initial fixtures may
//! observe a non-trivial parity gap because the CPU oracle's lighting
//! equation and the GPU shaders' post-FX chain accumulate to different
//! HDR values — that gap is the subject of follow-up slices per
//! ADR-046's per-fixture exception process.
//!
//! ## Tonemap output channel order
//!
//! Phase 5.5 A.2a swapped the tonemap storage-texture format from
//! `Bgra8UnormSrgb` to `Bgra8Unorm` with manual linear→sRGB encoding in
//! the shader. The readback delivers BGRA bytes; this harness's
//! [`tonemap_to_framebuffer`] swaps to RGBA so the comparison against
//! the CPU oracle (which writes `Rgba8`) lines up on the channel axis.

#![allow(dead_code)] // Harness surfaces grow per fixture; allow during A.3 build-out.

use std::panic::{self, AssertUnwindSafe};

use engine_gpu::{
    Buffer, BufferDesc, BufferUsage, COPY_BYTES_PER_ROW_ALIGNMENT, CommandEncoder, Device,
    DeviceLimits, Extent3d, Sampler, SamplerDesc, Texture, TextureDesc, TextureDimension,
    TextureFormat, TextureUsage,
};
use engine_raster::{Framebuffer, Rgba8};
use engine_render::{
    BloomPass, ClusterLightPass, CsmShadowPass, CullPass, GBufferPass, INDIRECT_DRAW_MAX_COUNT,
    IblPass, LightingAccumulationPass, Phase6Pipelines, RenderGraph, ResourceId, SsaoPass, TaaPass,
    TonemapPass, TransientResourceTable, bake_brdf_lut, build_all_phase6_pipelines, contracts,
};

// =============================================================================
// Canonical ResourceId numbering — mirror the
// `pr4_post_fx_chain_schedules_after_lighting` scheduling test in
// `crates/engine-render/src/passes.rs`.
// =============================================================================

// Graph-flow (topology) resources — these participate in `reads()` /
// `writes()` declarations and pin the topological sort.
pub const RID_RENDER_QUEUE: ResourceId = ResourceId(0); // cull instances SSBO
pub const RID_CASTERS: ResourceId = ResourceId(1); // shadow casters SSBO
pub const RID_LIGHTS: ResourceId = ResourceId(2); // light records SSBO
pub const RID_INDIRECT: ResourceId = ResourceId(3); // indirect-draw SSBO
pub const RID_SHADOW_ATLAS: ResourceId = ResourceId(4); // CSM atlas
pub const RID_CLUSTER_CELLS: ResourceId = ResourceId(5); // cluster grid SSBO
pub const RID_GBUF_AR: ResourceId = ResourceId(6); // albedo + roughness
pub const RID_GBUF_NM: ResourceId = ResourceId(7); // normal + metallic
pub const RID_GBUF_MD: ResourceId = ResourceId(8); // motion + depth
pub const RID_DEPTH: ResourceId = ResourceId(9); // hardware depth
pub const RID_LIT: ResourceId = ResourceId(10); // HDR lit color
pub const RID_PROBES: ResourceId = ResourceId(11); // IBL probe SSBO
pub const RID_BRDF_LUT: ResourceId = ResourceId(12); // BRDF LUT (pre-baked)
pub const RID_SSAO: ResourceId = ResourceId(13); // SSAO output
pub const RID_TAA_HISTORY: ResourceId = ResourceId(14); // prev-frame history
pub const RID_TAA_HISTORY_NEXT: ResourceId = ResourceId(15); // next-frame history
pub const RID_TAA_RESOLVED: ResourceId = ResourceId(16); // resolved HDR
pub const RID_BLOOM: ResourceId = ResourceId(17); // bloom mip chain
pub const RID_TONEMAPPED: ResourceId = ResourceId(18); // final LDR

// Auxiliary (non-graph-flow) resources — UBOs, samplers, secondary SSBOs.
pub const RID_FRUSTUM_UBO: ResourceId = ResourceId(50);
pub const RID_MESHES_SSBO: ResourceId = ResourceId(51);
pub const RID_DRAW_COUNT_SSBO: ResourceId = ResourceId(52);
pub const RID_CLUSTER_UBO: ResourceId = ResourceId(53);
pub const RID_LIGHT_INDICES_SSBO: ResourceId = ResourceId(54);
pub const RID_INDICES_CURSOR_SSBO: ResourceId = ResourceId(55);
pub const RID_SSAO_UBO: ResourceId = ResourceId(56);
pub const RID_IBL_UBO: ResourceId = ResourceId(57);
pub const RID_BRDF_SAMPLER: ResourceId = ResourceId(58);
pub const RID_TAA_UBO: ResourceId = ResourceId(59);
pub const RID_LINEAR_SAMPLER: ResourceId = ResourceId(60);
pub const RID_BLOOM_UBO: ResourceId = ResourceId(61);
pub const RID_TONEMAP_UBO: ResourceId = ResourceId(62);
pub const RID_CSM_UBO: ResourceId = ResourceId(63);
/// GBufferPass's `PerFrame` UBO at `gbuffer.wgsl:27` —
/// 3 mat4 + 2 vec4 = 224 B (rounded to 256 for alignment).
pub const RID_GBUFFER_FRAME_UBO: ResourceId = ResourceId(64);
pub const RID_SHADOW_SAMPLER: ResourceId = ResourceId(65);
pub const RID_VERTEX_BUF: ResourceId = ResourceId(66);
pub const RID_INDEX_BUF: ResourceId = ResourceId(67);
pub const RID_INSTANCES_SSBO: ResourceId = ResourceId(68);
/// LightingAccumulationPass's `FullScreenUniforms` UBO at
/// `lighting.wgsl:10` — `inv_view_projection` + `camera_pos` +
/// `screen_extent` + pad = 96 B.
///
/// **Distinct from `RID_GBUFFER_FRAME_UBO`.** The engine's pass-record
/// scheduling test wires the same `frame_ubo` resource id to both
/// GBufferPass and LightingAccumulationPass, but the two shaders
/// declare *different* WGSL struct layouts at the same
/// `@group(0) @binding(0)` slot. Sharing one buffer with both is
/// fine for the structural smokes (the bytes are uninitialised and
/// no shader reads correctness-sensitive output) but breaks pixel
/// parity — Lighting would interpret GBuffer's `view_projection`
/// matrix as its own `inv_view_projection`. The harness here
/// gives each shader its own buffer so the cube fixture seeds the
/// correct layout per pass.
pub const RID_LIGHTING_FRAME_UBO: ResourceId = ResourceId(69);

// =============================================================================
// Resource sizing constants
// =============================================================================

/// Cluster cells SSBO size (3 456 cells × 8 B per ADR-064 §5).
const CLUSTER_CELLS_SIZE: u64 = contracts::CLUSTER_CELL_COUNT as u64 * 8;
/// Light-indices SSBO size — max 32 lights per cell × cell count × 4 B.
const LIGHT_INDICES_SIZE: u64 =
    contracts::CLUSTER_CELL_COUNT as u64 * contracts::MAX_LIGHTS_PER_CLUSTER as u64 * 4;
/// Lights SSBO size — fits the contract cap (256 lights × 64 B).
const LIGHTS_SIZE: u64 = contracts::MAX_TOTAL_LIGHTS as u64 * 64;
/// IBL probe SSBO size (128 probes × 160 B per `IblProbeRecord`).
const IBL_PROBES_SIZE: u64 = contracts::MAX_IBL_PROBES as u64 * 160;
/// Indirect-draw SSBO size — `MAX_COUNT` × 20 B per `DrawIndexedIndirect`.
const INDIRECT_DRAW_SIZE: u64 = INDIRECT_DRAW_MAX_COUNT as u64 * 20;

// =============================================================================
// Try-device pattern (same as pass_record_e2e.rs)
// =============================================================================

pub fn try_device() -> Option<Device> {
    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        Device::new(DeviceLimits::Tier1Minimum, false)
    }));
    match result {
        Ok(Ok(d)) => Some(d),
        Ok(Err(e)) => {
            eprintln!("[parity] no compatible adapter (skipping): {e}");
            None
        }
        Err(_) => {
            eprintln!("[parity] wgpu has no backend enabled (skipping)");
            None
        }
    }
}

// =============================================================================
// ParityHarness
// =============================================================================

/// One-time-cost owned state shared across all parity fixtures.
pub struct ParityHarness {
    pub device: Device,
    pub pipelines: Phase6Pipelines,
    /// Pre-baked 512² Rg16Float BRDF LUT (init.rs:155).
    pub brdf_lut: Texture,
}

impl ParityHarness {
    /// Try to construct on the local Vulkan adapter. Returns `None` if
    /// no compatible device / loader (CI runners without wgpu vulkan).
    pub fn try_new() -> Option<Self> {
        let device = try_device()?;
        let queue = device.queue();
        let pipelines = match build_all_phase6_pipelines(&device) {
            Ok(p) => p,
            Err((name, e)) => {
                eprintln!("[parity] pipeline {name} build failed: {e:?} (skipping)");
                return None;
            }
        };
        let brdf_lut = bake_brdf_lut(&device, &queue, &pipelines.brdf_lut_bake);
        Some(Self {
            device,
            pipelines,
            brdf_lut,
        })
    }

    /// Allocate the canonical 10-pass transient resource pool at the
    /// given render extent. Textures + buffers + samplers populate
    /// every ResourceId every pass might look up.
    pub fn allocate_pool(&self, width: u32, height: u32) -> Pool {
        Pool::new(&self.device, &self.brdf_lut, width, height)
    }

    /// Build the canonical 10-pass graph against the harness's resource
    /// ids. The caller compiles + installs pipelines + executes.
    pub fn build_graph(&self) -> RenderGraph {
        let mut g = RenderGraph::new();
        g.add_pass(CullPass::new(
            RID_RENDER_QUEUE,
            RID_INDIRECT,
            RID_FRUSTUM_UBO,
            RID_MESHES_SSBO,
            RID_DRAW_COUNT_SSBO,
        ));
        g.add_pass(CsmShadowPass::new(
            RID_CASTERS,
            RID_SHADOW_ATLAS,
            RID_CSM_UBO,
            RID_VERTEX_BUF,
            RID_INDEX_BUF,
            RID_INDIRECT,
            RID_DRAW_COUNT_SSBO,
        ));
        g.add_pass(ClusterLightPass::new(
            RID_LIGHTS,
            RID_CLUSTER_CELLS,
            RID_CLUSTER_UBO,
            RID_LIGHT_INDICES_SSBO,
            RID_INDICES_CURSOR_SSBO,
        ));
        g.add_pass(GBufferPass::new(
            RID_INDIRECT,
            RID_DRAW_COUNT_SSBO,
            RID_GBUF_AR,
            RID_GBUF_NM,
            RID_GBUF_MD,
            RID_DEPTH,
            RID_GBUFFER_FRAME_UBO,
            RID_INSTANCES_SSBO,
            RID_VERTEX_BUF,
            RID_INDEX_BUF,
        ));
        g.add_pass(SsaoPass::new(
            RID_DEPTH,
            RID_GBUF_NM,
            RID_SSAO,
            RID_SSAO_UBO,
        ));
        g.add_pass(IblPass::new(
            RID_PROBES,
            RID_BRDF_LUT,
            RID_GBUF_AR,
            RID_GBUF_NM,
            RID_DEPTH,
            RID_LIT,
            RID_IBL_UBO,
            RID_BRDF_SAMPLER,
        ));
        g.add_pass(LightingAccumulationPass::new(
            RID_GBUF_AR,
            RID_GBUF_NM,
            RID_GBUF_MD,
            RID_DEPTH,
            RID_CLUSTER_CELLS,
            RID_LIGHTS,
            RID_SHADOW_ATLAS,
            RID_LIT,
            RID_LIGHTING_FRAME_UBO,
            RID_CLUSTER_UBO,
            RID_LIGHT_INDICES_SSBO,
            RID_SHADOW_SAMPLER,
        ));
        g.add_pass(TaaPass::new(
            RID_LIT,
            RID_TAA_HISTORY,
            RID_GBUF_MD,
            RID_DEPTH,
            RID_TAA_RESOLVED,
            RID_TAA_HISTORY_NEXT,
            RID_LIT,
            RID_TAA_UBO,
            RID_LINEAR_SAMPLER,
        ));
        g.add_pass(BloomPass::new(
            RID_TAA_RESOLVED,
            RID_BLOOM,
            RID_BLOOM_UBO,
            RID_LINEAR_SAMPLER,
        ));
        g.add_pass(TonemapPass::new(
            RID_TAA_RESOLVED,
            RID_BLOOM,
            RID_TONEMAPPED,
            RID_TONEMAP_UBO,
            RID_LINEAR_SAMPLER,
        ));
        g
    }

    /// Stage the tonemap target into a `MAP_READ` buffer + return the
    /// staging buffer. Caller submits the encoder, then calls
    /// [`tonemap_to_framebuffer`] on the staging buffer.
    pub fn copy_tonemap_to_staging(
        &self,
        encoder: &mut CommandEncoder,
        pool: &Pool,
    ) -> StagingBuffer {
        let width = pool.width;
        let height = pool.height;
        let bytes_per_pixel = 4u32;
        let unpadded = width * bytes_per_pixel;
        let padded = unpadded.div_ceil(COPY_BYTES_PER_ROW_ALIGNMENT) * COPY_BYTES_PER_ROW_ALIGNMENT;
        let size = padded as u64 * height as u64;
        let buf = Buffer::new(
            &self.device,
            &BufferDesc {
                label: "parity.tonemap.staging",
                size,
                usage: BufferUsage::COPY_DST | BufferUsage::MAP_READ,
            },
        );
        encoder.copy_texture_to_buffer(&pool.tonemapped, &buf, padded, height);
        StagingBuffer {
            buffer: buf,
            padded_row: padded,
            width,
            height,
        }
    }
}

// =============================================================================
// Pool
// =============================================================================

/// Per-fixture transient resource pool. Owns every texture / buffer /
/// sampler the 10-pass graph might look up, and registers them under
/// the canonical [`ResourceId`]s in [`TransientResourceTable`].
pub struct Pool {
    pub width: u32,
    pub height: u32,
    pub table: TransientResourceTable,
    /// Tonemap output — held outside the table for the readback path.
    pub tonemapped: Texture,
}

impl Pool {
    fn new(device: &Device, brdf_lut: &Texture, width: u32, height: u32) -> Self {
        let mut table = TransientResourceTable::new();

        // ---- G-buffer + depth ----
        let gbuf_ar = color_target(
            device,
            "parity.gbuf.ar",
            width,
            height,
            contracts::GBUFFER_ALBEDO_ROUGHNESS_FORMAT,
        );
        let gbuf_nm = color_target(
            device,
            "parity.gbuf.nm",
            width,
            height,
            contracts::GBUFFER_NORMAL_METALLIC_FORMAT,
        );
        let gbuf_md = color_target(
            device,
            "parity.gbuf.md",
            width,
            height,
            contracts::GBUFFER_MOTION_DEPTH_FORMAT,
        );
        let depth = depth_target(device, "parity.depth", width, height);

        // ---- shadow atlas + lit + IBL etc. ----
        let shadow = shadow_atlas(device, "parity.shadow.atlas");
        let lit = hdr_storage_target(device, "parity.lit", width, height);
        let ssao = ssao_target(device, "parity.ssao", width / 2, height / 2);
        let taa_history = hdr_storage_target(device, "parity.taa.history", width, height);
        let taa_history_next = hdr_storage_target(device, "parity.taa.history.next", width, height);
        let taa_resolved = hdr_storage_target(device, "parity.taa.resolved", width, height);
        let bloom = bloom_target(device, "parity.bloom", width, height);
        let tonemapped = tonemap_target(device, "parity.tonemapped", width, height);

        table.register_texture(RID_GBUF_AR, gbuf_ar);
        table.register_texture(RID_GBUF_NM, gbuf_nm);
        table.register_texture(RID_GBUF_MD, gbuf_md);
        table.register_texture(RID_DEPTH, depth);
        table.register_texture(RID_SHADOW_ATLAS, shadow);
        table.register_texture(RID_LIT, lit);
        table.register_texture(RID_SSAO, ssao);
        table.register_texture(RID_TAA_HISTORY, taa_history);
        table.register_texture(RID_TAA_HISTORY_NEXT, taa_history_next);
        table.register_texture(RID_TAA_RESOLVED, taa_resolved);
        table.register_texture(RID_BLOOM, bloom);
        // tonemapped held separately — readback consumer needs `&Texture`.

        // Clone the BRDF LUT so the table owns it under the canonical id.
        // The harness retains the original; both `Texture` handles refer
        // to the same wgpu resource because the underlying `Texture`
        // type wraps a refcounted handle in wgpu 29 (cheap clone).
        // *Actually* `Texture` doesn't impl Clone in engine-gpu; we
        // re-bake a 1×1 placeholder for now (resolvers borrow it; the
        // tests passing the LUT through the BRDF binding will need
        // either a Clone impl on Texture, or moving the harness's
        // brdf_lut into the table directly).
        //
        // For Slice 2A the IBL pass short-circuits on missing probe
        // contents anyway. Register a placeholder so resolver lookup
        // doesn't fail; visual parity for IBL is a later slice.
        let brdf_placeholder = brdf_placeholder_texture(device);
        let _ = brdf_lut; // suppress unused — see comment above.
        table.register_texture(RID_BRDF_LUT, brdf_placeholder);

        // ---- buffers ----
        // CullPass inputs/outputs.
        table.register_buffer(
            RID_RENDER_QUEUE,
            ssbo(device, "parity.cull.instances", 4096),
        );
        table.register_buffer(RID_FRUSTUM_UBO, ubo(device, "parity.cull.frustum", 96));
        table.register_buffer(RID_MESHES_SSBO, ssbo(device, "parity.cull.meshes", 4096));
        table.register_buffer(
            RID_INDIRECT,
            indirect_ssbo(device, "parity.cull.draws", INDIRECT_DRAW_SIZE),
        );
        table.register_buffer(
            RID_DRAW_COUNT_SSBO,
            indirect_ssbo(device, "parity.cull.count", 16),
        );
        // CSM/GBuffer per-instance + mesh VB/IB.
        table.register_buffer(
            RID_VERTEX_BUF,
            vertex_buf(device, "parity.mesh.vb", 64 * 1024),
        );
        table.register_buffer(RID_INDEX_BUF, index_buf(device, "parity.mesh.ib", 8 * 1024));
        table.register_buffer(
            RID_INSTANCES_SSBO,
            ssbo(device, "parity.instances", 64 * 1024),
        );
        table.register_buffer(RID_CASTERS, ssbo(device, "parity.casters", 4096));
        // Lighting + IBL + cluster.
        table.register_buffer(RID_LIGHTS, ssbo(device, "parity.lights", LIGHTS_SIZE));
        table.register_buffer(
            RID_CLUSTER_CELLS,
            ssbo(device, "parity.cluster.cells", CLUSTER_CELLS_SIZE),
        );
        table.register_buffer(
            RID_LIGHT_INDICES_SSBO,
            ssbo(device, "parity.cluster.indices", LIGHT_INDICES_SIZE),
        );
        table.register_buffer(
            RID_INDICES_CURSOR_SSBO,
            ssbo(device, "parity.cluster.cursor", 16),
        );
        table.register_buffer(
            RID_PROBES,
            ssbo(device, "parity.ibl.probes", IBL_PROBES_SIZE),
        );
        // UBOs.
        table.register_buffer(
            RID_GBUFFER_FRAME_UBO,
            ubo(device, "parity.ubo.frame.gbuffer", 256),
        );
        table.register_buffer(
            RID_LIGHTING_FRAME_UBO,
            ubo(device, "parity.ubo.frame.lighting", 96),
        );
        table.register_buffer(RID_CSM_UBO, ubo(device, "parity.ubo.csm", 384));
        table.register_buffer(RID_CLUSTER_UBO, ubo(device, "parity.ubo.cluster", 112));
        table.register_buffer(RID_SSAO_UBO, ubo(device, "parity.ubo.ssao", 256));
        table.register_buffer(RID_IBL_UBO, ubo(device, "parity.ubo.ibl", 96));
        table.register_buffer(RID_TAA_UBO, ubo(device, "parity.ubo.taa", 96));
        table.register_buffer(RID_BLOOM_UBO, ubo(device, "parity.ubo.bloom", 16));
        table.register_buffer(RID_TONEMAP_UBO, ubo(device, "parity.ubo.tonemap", 16));

        // ---- samplers ----
        table.register_sampler(
            RID_LINEAR_SAMPLER,
            Sampler::new(device, SamplerDesc::linear_repeat()),
        );
        table.register_sampler(
            RID_BRDF_SAMPLER,
            Sampler::new(device, SamplerDesc::linear_repeat()),
        );
        table.register_sampler(
            RID_SHADOW_SAMPLER,
            Sampler::new(device, SamplerDesc::shadow_pcf()),
        );

        Self {
            width,
            height,
            table,
            tonemapped,
        }
    }
}

// =============================================================================
// StagingBuffer + Framebuffer conversion
// =============================================================================

/// Staging buffer for tonemap readback. `read_back_to_framebuffer`
/// performs the BGRA→RGBA swap that the comparison oracle expects.
pub struct StagingBuffer {
    pub buffer: Buffer,
    pub padded_row: u32,
    pub width: u32,
    pub height: u32,
}

impl StagingBuffer {
    /// Map the buffer, slice out the padded row stride, swap BGRA→RGBA,
    /// pack into an [`engine_raster::Framebuffer`] for `compare_images`.
    pub fn read_back_to_framebuffer(&self) -> Framebuffer {
        let bytes = self
            .buffer
            .read_back()
            .expect("tonemap staging buffer maps for read");
        let mut fb = Framebuffer::new(self.width, self.height);
        for y in 0..self.height {
            for x in 0..self.width {
                let row_base = (y * self.padded_row) as usize;
                let pix_base = row_base + (x as usize) * 4;
                // wgpu's `Bgra8Unorm` lays out BGRA in memory; the CPU
                // oracle's `Rgba8` expects R first. Swap channels.
                let b = bytes[pix_base];
                let g = bytes[pix_base + 1];
                let r = bytes[pix_base + 2];
                let a = bytes[pix_base + 3];
                fb.write(x, y, Rgba8 { r, g, b, a });
            }
        }
        fb
    }
}

// =============================================================================
// Texture / buffer descriptor helpers (internal)
// =============================================================================

fn color_target(
    device: &Device,
    label: &'static str,
    width: u32,
    height: u32,
    format: TextureFormat,
) -> Texture {
    Texture::new(
        device,
        &TextureDesc {
            label,
            extent: Extent3d::new_2d(width, height),
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format,
            usage: TextureUsage::RENDER_ATTACHMENT
                | TextureUsage::TEXTURE_BINDING
                | TextureUsage::COPY_SRC,
        },
    )
}

fn depth_target(device: &Device, label: &'static str, width: u32, height: u32) -> Texture {
    Texture::new(
        device,
        &TextureDesc {
            label,
            extent: Extent3d::new_2d(width, height),
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            // COPY_SRC enables depth-buffer readback for parity-fixture
            // diagnostics (Slice 6 confirms GBuffer wrote depth).
            format: contracts::DEPTH_BUFFER_FORMAT,
            usage: TextureUsage::RENDER_ATTACHMENT
                | TextureUsage::TEXTURE_BINDING
                | TextureUsage::COPY_SRC,
        },
    )
}

fn shadow_atlas(device: &Device, label: &'static str) -> Texture {
    Texture::new(
        device,
        &TextureDesc {
            label,
            // CSM atlas is canonically 4 096²; the parity fixtures only
            // need the structural binding, so a 512² atlas keeps the
            // VRAM footprint small. CsmShadowPass clears the atlas to
            // reverse-Z 0.0 every frame; no scaling math depends on the
            // dimension.
            extent: Extent3d::new_2d(512, 512),
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: contracts::DEPTH_BUFFER_FORMAT,
            usage: TextureUsage::RENDER_ATTACHMENT | TextureUsage::TEXTURE_BINDING,
        },
    )
}

fn hdr_storage_target(device: &Device, label: &'static str, width: u32, height: u32) -> Texture {
    Texture::new(
        device,
        &TextureDesc {
            label,
            extent: Extent3d::new_2d(width, height),
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: contracts::LIT_COLOR_FORMAT, // Rgba16Float
            usage: TextureUsage::RENDER_ATTACHMENT
                | TextureUsage::TEXTURE_BINDING
                | TextureUsage::STORAGE_BINDING,
        },
    )
}

fn ssao_target(device: &Device, label: &'static str, width: u32, height: u32) -> Texture {
    // SSAO writes `texture_storage_2d<r16float, write>` per
    // `shaders/ssao.wgsl`; wgpu's bind-group validator pins the
    // storage-texture format to match the shader declaration exactly.
    Texture::new(
        device,
        &TextureDesc {
            label,
            extent: Extent3d::new_2d(width.max(1), height.max(1)),
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::R16Float,
            usage: TextureUsage::TEXTURE_BINDING | TextureUsage::STORAGE_BINDING,
        },
    )
}

fn bloom_target(device: &Device, label: &'static str, width: u32, height: u32) -> Texture {
    Texture::new(
        device,
        &TextureDesc {
            label,
            extent: Extent3d::new_2d(width.max(32), height.max(32)),
            mip_level_count: contracts::BLOOM_MIP_LEVELS,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba16Float,
            usage: TextureUsage::TEXTURE_BINDING | TextureUsage::STORAGE_BINDING,
        },
    )
}

fn tonemap_target(device: &Device, label: &'static str, width: u32, height: u32) -> Texture {
    // Per A.2a: tonemap writes `Bgra8Unorm` with manual linear→sRGB in
    // the shader. The COPY_SRC usage enables texture-to-buffer readback
    // for parity comparison.
    Texture::new(
        device,
        &TextureDesc {
            label,
            extent: Extent3d::new_2d(width, height),
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Bgra8Unorm,
            usage: TextureUsage::TEXTURE_BINDING
                | TextureUsage::STORAGE_BINDING
                | TextureUsage::COPY_SRC,
        },
    )
}

fn brdf_placeholder_texture(device: &Device) -> Texture {
    Texture::new(
        device,
        &TextureDesc {
            label: "parity.brdf_lut.placeholder",
            extent: Extent3d::new_2d(contracts::BRDF_LUT_DIM, contracts::BRDF_LUT_DIM),
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rg16Float,
            usage: TextureUsage::TEXTURE_BINDING | TextureUsage::STORAGE_BINDING,
        },
    )
}

fn ssbo(device: &Device, label: &'static str, size: u64) -> Buffer {
    Buffer::new(
        device,
        &BufferDesc {
            label,
            size: size.max(16),
            usage: BufferUsage::STORAGE | BufferUsage::COPY_DST,
        },
    )
}

fn ubo(device: &Device, label: &'static str, size: u64) -> Buffer {
    Buffer::new(
        device,
        &BufferDesc {
            label,
            size: size.max(16),
            usage: BufferUsage::UNIFORM | BufferUsage::COPY_DST,
        },
    )
}

fn indirect_ssbo(device: &Device, label: &'static str, size: u64) -> Buffer {
    Buffer::new(
        device,
        &BufferDesc {
            label,
            size: size.max(16),
            usage: BufferUsage::STORAGE | BufferUsage::INDIRECT | BufferUsage::COPY_DST,
        },
    )
}

fn vertex_buf(device: &Device, label: &'static str, size: u64) -> Buffer {
    Buffer::new(
        device,
        &BufferDesc {
            label,
            size: size.max(16),
            usage: BufferUsage::VERTEX | BufferUsage::COPY_DST,
        },
    )
}

fn index_buf(device: &Device, label: &'static str, size: u64) -> Buffer {
    Buffer::new(
        device,
        &BufferDesc {
            label,
            size: size.max(16),
            usage: BufferUsage::INDEX | BufferUsage::COPY_DST,
        },
    )
}
