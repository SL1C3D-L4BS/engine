//! Command-encoder + render-pass + compute-pass wrappers.
//!
//! Phase 5.5 A.2b extends the surface with MRT + depth attachments,
//! `set_bind_group`, `set_push_constants`, and `draw_indexed_indirect`
//! so the Track-A passes can record real GPU work.

use crate::buffer::Buffer;
use crate::device::Device;
use crate::pipeline::{BindGroup, ComputePipeline, RenderPipeline};
use crate::texture::{Texture, TextureView};

/// RGBA clear value for a [`RenderPassColorAttachment`].
#[derive(Clone, Copy, Debug, Default)]
pub struct Color {
    /// Red component (linear).
    pub r: f64,
    /// Green component (linear).
    pub g: f64,
    /// Blue component (linear).
    pub b: f64,
    /// Alpha component.
    pub a: f64,
}

impl Color {
    /// Opaque black.
    pub const BLACK: Color = Color {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    };
    /// Fully transparent.
    pub const TRANSPARENT: Color = Color {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 0.0,
    };

    /// Construct from r/g/b/a components.
    pub const fn new(r: f64, g: f64, b: f64, a: f64) -> Self {
        Self { r, g, b, a }
    }
}

/// Color-attachment load operation.
#[derive(Clone, Copy, Debug)]
pub enum LoadOp {
    /// Clear the attachment to the given color before the pass begins.
    Clear(Color),
    /// Preserve the existing attachment contents.
    Load,
}

/// Depth-attachment load operation.
#[derive(Clone, Copy, Debug)]
pub enum DepthLoadOp {
    /// Clear depth to the given value (reverse-Z uses `0.0` at the far
    /// plane; conventional Z uses `1.0`).
    Clear(f32),
    /// Preserve the existing depth contents.
    Load,
}

/// One color attachment in a [`RenderPassDesc`].
#[derive(Clone, Debug)]
pub struct RenderPassColorAttachment<'a> {
    /// Color target view.
    pub view: &'a TextureView<'a>,
    /// Load operation.
    pub load: LoadOp,
    /// `true` to keep the result after the pass; `false` discards.
    pub store: bool,
}

/// Depth attachment in a [`RenderPassDesc`].
#[derive(Clone, Debug)]
pub struct RenderPassDepthAttachment<'a> {
    /// Depth target view.
    pub view: &'a TextureView<'a>,
    /// Load operation.
    pub load: DepthLoadOp,
    /// `true` to keep the result after the pass; `false` discards.
    pub store: bool,
}

/// Multi-target render-pass descriptor.
#[derive(Clone, Debug)]
pub struct RenderPassDesc<'a> {
    /// Trace label.
    pub label: &'a str,
    /// Color attachments (1..=8 typically). Empty for depth-only passes.
    pub color_attachments: &'a [RenderPassColorAttachment<'a>],
    /// Optional depth attachment.
    pub depth: Option<RenderPassDepthAttachment<'a>>,
}

/// Owned command encoder.
///
/// Records commands into one or more passes; the finished
/// `wgpu::CommandBuffer` is consumed by [`crate::Queue::submit`].
#[derive(Debug)]
pub struct CommandEncoder {
    raw: wgpu::CommandEncoder,
}

impl CommandEncoder {
    /// Create a new encoder bound to `device`.
    pub fn new(device: &Device, label: &str) -> Self {
        let raw = device
            .raw()
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some(label) });
        Self { raw }
    }

    /// Begin a colour-only render pass. The single attachment is cleared to
    /// `clear` and stored. Shortcut for the most common case;
    /// [`Self::begin_render_pass_desc`] is the MRT + depth surface.
    pub fn begin_render_pass<'a>(
        &'a mut self,
        label: &'a str,
        view: TextureView<'a>,
        clear: [f64; 4],
    ) -> RenderPass<'a> {
        let raw = self.raw.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(label),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: view.raw(),
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: clear[0],
                        g: clear[1],
                        b: clear[2],
                        a: clear[3],
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        RenderPass { raw }
    }

    /// Begin a multi-target render pass with optional depth attachment.
    pub fn begin_render_pass_desc<'a>(&'a mut self, desc: &RenderPassDesc<'a>) -> RenderPass<'a> {
        let color_attachments: Vec<Option<wgpu::RenderPassColorAttachment<'_>>> = desc
            .color_attachments
            .iter()
            .map(|a| {
                Some(wgpu::RenderPassColorAttachment {
                    view: a.view.raw(),
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: match a.load {
                            LoadOp::Clear(c) => wgpu::LoadOp::Clear(wgpu::Color {
                                r: c.r,
                                g: c.g,
                                b: c.b,
                                a: c.a,
                            }),
                            LoadOp::Load => wgpu::LoadOp::Load,
                        },
                        store: if a.store {
                            wgpu::StoreOp::Store
                        } else {
                            wgpu::StoreOp::Discard
                        },
                    },
                })
            })
            .collect();
        let depth_stencil_attachment =
            desc.depth
                .as_ref()
                .map(|d| wgpu::RenderPassDepthStencilAttachment {
                    view: d.view.raw(),
                    depth_ops: Some(wgpu::Operations {
                        load: match d.load {
                            DepthLoadOp::Clear(v) => wgpu::LoadOp::Clear(v),
                            DepthLoadOp::Load => wgpu::LoadOp::Load,
                        },
                        store: if d.store {
                            wgpu::StoreOp::Store
                        } else {
                            wgpu::StoreOp::Discard
                        },
                    }),
                    stencil_ops: None,
                });
        let raw = self.raw.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(desc.label),
            color_attachments: &color_attachments,
            depth_stencil_attachment,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        RenderPass { raw }
    }

    /// Begin a compute pass.
    pub fn begin_compute_pass<'a>(&'a mut self, label: &'a str) -> ComputePass<'a> {
        let raw = self.raw.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some(label),
            timestamp_writes: None,
        });
        ComputePass { raw }
    }

    /// Copy a contiguous byte range from `src` to `dst`. Both must have
    /// matching [`crate::BufferUsage::COPY_SRC`] / `COPY_DST`.
    pub fn copy_buffer_to_buffer(
        &mut self,
        src: &Buffer,
        src_offset: u64,
        dst: &Buffer,
        dst_offset: u64,
        size: u64,
    ) {
        self.raw
            .copy_buffer_to_buffer(src.raw(), src_offset, dst.raw(), dst_offset, size);
    }

    /// Copy the mip-0 base-layer footprint of `src` into `dst` starting at
    /// byte offset 0. Symmetric with [`crate::Queue::write_texture_2d`]:
    /// uploads write into the texture; this method downloads back out.
    ///
    /// `bytes_per_row` must align with [`crate::COPY_BYTES_PER_ROW_ALIGNMENT`]
    /// (256 on native backends). For an `Rgba8` / `Bgra8` texture of width
    /// `W`, the row pitch is `((W * 4 + 255) / 256) * 256`; the buffer must
    /// be sized for at least `bytes_per_row * rows_per_image` bytes. Callers
    /// are responsible for unpacking the padded rows on the host side after
    /// [`Buffer::read_back`] returns.
    ///
    /// `src` must have been created with [`crate::TextureUsage::COPY_SRC`];
    /// `dst` with [`crate::BufferUsage::COPY_DST`] (and typically
    /// `MAP_READ` for readback).
    pub fn copy_texture_to_buffer(
        &mut self,
        src: &Texture,
        dst: &Buffer,
        bytes_per_row: u32,
        rows_per_image: u32,
    ) {
        let extent = src.extent();
        self.raw.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: src.raw(),
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: dst.raw(),
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(rows_per_image),
                },
            },
            wgpu::Extent3d {
                width: extent.width,
                height: extent.height,
                depth_or_array_layers: extent.depth_or_array_layers,
            },
        );
    }

    /// Consume the encoder, returning the underlying `wgpu::CommandBuffer`.
    /// Crate-internal; the public exit is [`crate::Queue::submit`].
    pub(crate) fn finish_raw(self) -> wgpu::CommandBuffer {
        self.raw.finish()
    }
}

/// In-flight render pass.
///
/// PR 2 exposes: set pipeline, set vertex buffer, set index buffer, draw,
/// draw-indexed. PR 3+ adds bind-group setters, push-constants, indirect
/// draws.
pub struct RenderPass<'a> {
    raw: wgpu::RenderPass<'a>,
}

impl RenderPass<'_> {
    /// Bind a render pipeline.
    pub fn set_pipeline(&mut self, pipeline: &RenderPipeline) {
        self.raw.set_pipeline(pipeline.raw());
    }

    /// Bind a bind group to slot `slot`. Dynamic offsets are not yet
    /// surfaced — Phase 5.5 passes use static offsets only.
    pub fn set_bind_group(&mut self, slot: u32, bind_group: &BindGroup) {
        self.raw.set_bind_group(slot, bind_group.raw(), &[]);
    }

    /// Write `bytes` into the pipeline's immediate (push-constant) data
    /// at the given byte offset. Stage visibility is baked into the
    /// pipeline layout's immediate range (wgpu 29 dropped per-call
    /// stage flags). ADR-044 keeps the engine method name
    /// `set_push_constants` for spec stability; wgpu 29 calls the
    /// primitive `set_immediates`.
    pub fn set_push_constants(&mut self, offset: u32, bytes: &[u8]) {
        self.raw.set_immediates(offset, bytes);
    }

    /// Bind a vertex buffer to slot `slot`.
    pub fn set_vertex_buffer(&mut self, slot: u32, buffer: &Buffer) {
        self.raw.set_vertex_buffer(slot, buffer.raw().slice(..));
    }

    /// Bind a u32 index buffer.
    pub fn set_index_buffer_u32(&mut self, buffer: &Buffer) {
        self.raw
            .set_index_buffer(buffer.raw().slice(..), wgpu::IndexFormat::Uint32);
    }

    /// Issue a non-indexed draw.
    pub fn draw(&mut self, vertices: core::ops::Range<u32>, instances: core::ops::Range<u32>) {
        self.raw.draw(vertices, instances);
    }

    /// Issue an indexed draw.
    pub fn draw_indexed(
        &mut self,
        indices: core::ops::Range<u32>,
        base_vertex: i32,
        instances: core::ops::Range<u32>,
    ) {
        self.raw.draw_indexed(indices, base_vertex, instances);
    }

    /// Issue a single indirect indexed draw — the GPU reads the
    /// `DrawIndexedIndirect { index_count, instance_count, first_index,
    /// base_vertex, first_instance }` argument struct from the buffer
    /// at the given offset. The `CullPass` produces this argument buffer.
    pub fn draw_indexed_indirect(&mut self, buffer: &Buffer, offset: u64) {
        self.raw.draw_indexed_indirect(buffer.raw(), offset);
    }

    /// Issue up to `max_count` indirect indexed draws — the GPU reads
    /// `count_buffer[count_offset..count_offset+4]` as a `u32` for the
    /// actual draw count and then consumes that many
    /// `DrawIndexedIndirect` structs from `indirect_buffer` starting at
    /// `indirect_offset`. The `CullPass` produces the
    /// `indirect_buffer` (per-survivor `DrawIndexedIndirect`) + the
    /// `count_buffer` (atomic `u32`).
    ///
    /// Requires [`crate::DeviceFeatures::multi_draw_indirect_count`].
    /// Callers must short-circuit when the device does not advertise it
    /// (`Polaris/RADV` advertises it; older hosts do not — the affected
    /// pass falls back to its A.2c clear-only path).
    pub fn multi_draw_indexed_indirect_count(
        &mut self,
        indirect_buffer: &Buffer,
        indirect_offset: u64,
        count_buffer: &Buffer,
        count_offset: u64,
        max_count: u32,
    ) {
        self.raw.multi_draw_indexed_indirect_count(
            indirect_buffer.raw(),
            indirect_offset,
            count_buffer.raw(),
            count_offset,
            max_count,
        );
    }
}

impl core::fmt::Debug for RenderPass<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RenderPass").finish_non_exhaustive()
    }
}

/// In-flight compute pass.
pub struct ComputePass<'a> {
    raw: wgpu::ComputePass<'a>,
}

impl ComputePass<'_> {
    /// Bind a compute pipeline.
    pub fn set_pipeline(&mut self, pipeline: &ComputePipeline) {
        self.raw.set_pipeline(pipeline.raw());
    }

    /// Bind a bind group to slot `slot`.
    pub fn set_bind_group(&mut self, slot: u32, bind_group: &BindGroup) {
        self.raw.set_bind_group(slot, bind_group.raw(), &[]);
    }

    /// Write `bytes` into the pipeline's immediate (push-constant) data
    /// at the given byte offset. ADR-044 keeps the engine method name
    /// `set_push_constants` for spec stability; wgpu 29 calls the
    /// primitive `set_immediates`.
    pub fn set_push_constants(&mut self, offset: u32, bytes: &[u8]) {
        self.raw.set_immediates(offset, bytes);
    }

    /// Dispatch `groups` workgroups.
    pub fn dispatch_workgroups(&mut self, x: u32, y: u32, z: u32) {
        self.raw.dispatch_workgroups(x, y, z);
    }
}

impl core::fmt::Debug for ComputePass<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ComputePass").finish_non_exhaustive()
    }
}

/// Token returned by [`crate::Queue::submit`].
///
/// Wraps wgpu's `SubmissionIndex` so callers don't reach for the wgpu type;
/// PR 3+ exposes a `wait` method when fence-based readback is needed.
#[derive(Debug)]
pub struct SubmitToken {
    #[allow(dead_code, reason = "consumed by PR 3 fence-wait")]
    raw: wgpu::SubmissionIndex,
}

impl SubmitToken {
    pub(crate) fn new(raw: wgpu::SubmissionIndex) -> Self {
        Self { raw }
    }

    /// Borrow the underlying `wgpu::SubmissionIndex`. Crate-internal —
    /// consumed by PR 3's fence-wait surface.
    #[allow(dead_code, reason = "consumed by PR 3 fence-wait")]
    pub(crate) fn raw(&self) -> &wgpu::SubmissionIndex {
        &self.raw
    }
}
