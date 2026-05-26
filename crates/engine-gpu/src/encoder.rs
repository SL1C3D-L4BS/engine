//! Command-encoder + render-pass + compute-pass wrappers.
//!
//! PR 2's pass surface is intentionally narrow: enough verbs that PR 3's
//! deferred geometry pass and the rasterizer testbed's GPU backend can
//! record real work, no more. New verbs land alongside their consumer.

use crate::buffer::Buffer;
use crate::device::Device;
use crate::pipeline::{ComputePipeline, RenderPipeline};
use crate::texture::TextureView;

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
    /// `clear` and stored. PR 3+ widens the descriptor to MRT.
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
