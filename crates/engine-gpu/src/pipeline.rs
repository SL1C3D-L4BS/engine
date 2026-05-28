//! Pipeline / shader-module / bind-group layout wrappers.
//!
//! Owned descriptors mirror the wgpu shape closely so PR 3+ render passes
//! can be authored in a familiar style, but every named identifier is
//! engine-owned. PR 2 ships the surface; PR 3 lights it up.

use crate::device::Device;
use crate::texture::TextureFormat;
use std::sync::Arc;

/// Shader module descriptor. PR 2 accepts only WGSL source strings — the
/// Slang toolchain (ADR-037) emits both WGSL (web) and SPIR-V (native), and
/// the SPIR-V variant lands in PR 3 alongside the geometry pass that
/// consumes it.
#[derive(Clone, Debug)]
pub struct ShaderModuleDesc<'a> {
    /// Debug label.
    pub label: &'a str,
    /// WGSL source.
    pub wgsl: &'a str,
}

/// Owned shader module. Clone is cheap (Arc-backed) — render-pipeline
/// descriptors share one module across multiple stages.
#[derive(Clone, Debug)]
pub struct ShaderModule {
    raw: Arc<wgpu::ShaderModule>,
}

impl ShaderModule {
    /// Compile WGSL into a shader module.
    pub fn new(device: &Device, desc: &ShaderModuleDesc<'_>) -> Self {
        let raw = device
            .raw()
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(desc.label),
                source: wgpu::ShaderSource::Wgsl(desc.wgsl.into()),
            });
        Self { raw: Arc::new(raw) }
    }

    fn raw(&self) -> &wgpu::ShaderModule {
        &self.raw
    }
}

/// Shader stage flags. PR 2's pipelines only need vertex / fragment /
/// compute; geometry / tessellation are not engine surfaces (the spec
/// rejects them — Track A is straight raster).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShaderStage(u32);

impl ShaderStage {
    /// Vertex stage.
    pub const VERTEX: ShaderStage = ShaderStage(1 << 0);
    /// Fragment stage.
    pub const FRAGMENT: ShaderStage = ShaderStage(1 << 1);
    /// Compute stage.
    pub const COMPUTE: ShaderStage = ShaderStage(1 << 2);

    /// Set union.
    pub const fn union(self, other: ShaderStage) -> ShaderStage {
        ShaderStage(self.0 | other.0)
    }

    /// Membership test.
    pub const fn contains(self, other: ShaderStage) -> bool {
        (self.0 & other.0) == other.0
    }

    #[allow(dead_code, reason = "consumed by PR 3 bind-group entries")]
    fn to_wgpu(self) -> wgpu::ShaderStages {
        let mut s = wgpu::ShaderStages::empty();
        if self.contains(Self::VERTEX) {
            s |= wgpu::ShaderStages::VERTEX;
        }
        if self.contains(Self::FRAGMENT) {
            s |= wgpu::ShaderStages::FRAGMENT;
        }
        if self.contains(Self::COMPUTE) {
            s |= wgpu::ShaderStages::COMPUTE;
        }
        s
    }
}

impl core::ops::BitOr for ShaderStage {
    type Output = ShaderStage;
    fn bitor(self, rhs: Self) -> Self::Output {
        self.union(rhs)
    }
}

/// Bind-group layout descriptor. PR 2 ships the minimal surface PR 3 needs.
#[derive(Clone, Debug, Default)]
pub struct BindGroupLayoutDesc<'a> {
    /// Debug label.
    pub label: &'a str,
}

/// Owned bind-group layout.
#[derive(Clone, Debug)]
pub struct BindGroupLayout {
    raw: Arc<wgpu::BindGroupLayout>,
}

impl BindGroupLayout {
    /// Create an empty bind-group layout. PR 3+ will accept full entry
    /// lists; PR 2 only needs the empty layout to construct PipelineLayout.
    pub fn new(device: &Device, desc: &BindGroupLayoutDesc<'_>) -> Self {
        let raw = device
            .raw()
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some(desc.label),
                entries: &[],
            });
        Self { raw: Arc::new(raw) }
    }

    fn raw(&self) -> &wgpu::BindGroupLayout {
        &self.raw
    }
}

/// Pipeline-layout descriptor.
#[derive(Clone, Debug)]
pub struct PipelineLayoutDesc<'a> {
    /// Debug label.
    pub label: &'a str,
    /// Bind-group layouts (in set-index order).
    pub bind_group_layouts: &'a [&'a BindGroupLayout],
}

/// Owned pipeline layout.
#[derive(Clone, Debug)]
pub struct PipelineLayout {
    raw: Arc<wgpu::PipelineLayout>,
}

impl PipelineLayout {
    /// Create a pipeline layout from a list of bind-group layouts.
    pub fn new(device: &Device, desc: &PipelineLayoutDesc<'_>) -> Self {
        // wgpu 29: `bind_group_layouts: &[Option<&BindGroupLayout>]`. Every
        // engine entry is `Some(...)` — unbound slots are not used. The
        // `push_constant_ranges` field was replaced by a flat
        // `immediate_size: u32` byte budget; ADR-044 §6 specifies 8 bytes
        // per draw (texture_id, sampler_id), so we set 8 unconditionally
        // and rely on `Features::IMMEDIATES` being requested at device
        // create time when push-constants are available.
        let bgl_storage: Vec<Option<&wgpu::BindGroupLayout>> = desc
            .bind_group_layouts
            .iter()
            .map(|b| Some(b.raw()))
            .collect();
        let raw = device
            .raw()
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(desc.label),
                bind_group_layouts: &bgl_storage,
                immediate_size: 8,
            });
        Self { raw: Arc::new(raw) }
    }

    fn raw(&self) -> &wgpu::PipelineLayout {
        &self.raw
    }
}

/// Vertex attribute scalar / vector type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VertexFormat {
    /// 32-bit float.
    Float32,
    /// 32-bit float ×2.
    Float32x2,
    /// 32-bit float ×3.
    Float32x3,
    /// 32-bit float ×4.
    Float32x4,
    /// Unsigned 32-bit integer.
    Uint32,
    /// Unsigned 32-bit integer ×4.
    Uint32x4,
}

impl VertexFormat {
    fn to_wgpu(self) -> wgpu::VertexFormat {
        match self {
            VertexFormat::Float32 => wgpu::VertexFormat::Float32,
            VertexFormat::Float32x2 => wgpu::VertexFormat::Float32x2,
            VertexFormat::Float32x3 => wgpu::VertexFormat::Float32x3,
            VertexFormat::Float32x4 => wgpu::VertexFormat::Float32x4,
            VertexFormat::Uint32 => wgpu::VertexFormat::Uint32,
            VertexFormat::Uint32x4 => wgpu::VertexFormat::Uint32x4,
        }
    }
}

/// Vertex-rate stepping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VertexStepMode {
    /// Per vertex.
    Vertex,
    /// Per instance.
    Instance,
}

impl VertexStepMode {
    fn to_wgpu(self) -> wgpu::VertexStepMode {
        match self {
            VertexStepMode::Vertex => wgpu::VertexStepMode::Vertex,
            VertexStepMode::Instance => wgpu::VertexStepMode::Instance,
        }
    }
}

/// Single vertex attribute.
#[derive(Clone, Copy, Debug)]
pub struct VertexAttribute {
    /// Byte offset within the per-vertex stride.
    pub offset: u64,
    /// Shader-side location.
    pub shader_location: u32,
    /// Attribute format.
    pub format: VertexFormat,
}

/// One vertex-buffer layout (a contiguous interleaved buffer).
#[derive(Clone, Debug)]
pub struct VertexBufferLayout<'a> {
    /// Per-vertex byte stride.
    pub array_stride: u64,
    /// Step mode.
    pub step_mode: VertexStepMode,
    /// Attribute list.
    pub attributes: &'a [VertexAttribute],
}

/// Vertex stage descriptor.
#[derive(Clone, Debug)]
pub struct VertexState<'a> {
    /// Compiled vertex shader.
    pub module: &'a ShaderModule,
    /// Entry point name (e.g. `"vs_main"`).
    pub entry_point: &'a str,
    /// Vertex buffer layouts.
    pub buffers: &'a [VertexBufferLayout<'a>],
}

/// Fragment color-target description.
#[derive(Clone, Copy, Debug)]
pub struct ColorTargetState {
    /// Render-target format.
    pub format: TextureFormat,
}

/// Fragment stage descriptor.
#[derive(Clone, Debug)]
pub struct FragmentState<'a> {
    /// Compiled fragment shader.
    pub module: &'a ShaderModule,
    /// Entry point name (e.g. `"fs_main"`).
    pub entry_point: &'a str,
    /// Colour targets.
    pub targets: &'a [ColorTargetState],
}

/// Depth-stencil descriptor.
#[derive(Clone, Copy, Debug)]
pub struct DepthStencilState {
    /// Depth-buffer format.
    pub format: TextureFormat,
    /// Whether depth writes are enabled.
    pub depth_write_enabled: bool,
}

/// Render-pipeline descriptor.
///
/// `layout` is `Option<&PipelineLayout>` so the engine can choose between
/// (a) an explicit, hand-authored layout (the production path for ADR-075
/// `bindgroups/`-module discipline) and (b) wgpu's auto-derive
/// (`None`) which introspects the shader's `@group`/`@binding`/
/// `var<push_constant>` declarations and synthesises an implicit
/// layout. The implicit layout is queryable per-set via
/// [`RenderPipeline::bind_group_layout`].
///
/// ADR-075 §8 specifies that A.2a uses auto-derive as the unblock for
/// the smoke test; explicit layouts replace auto-derive on a per-pass
/// basis as the `bindgroups/` modules land in A.2b / A.2c.
#[derive(Clone, Debug)]
pub struct RenderPipelineDesc<'a> {
    /// Debug label.
    pub label: &'a str,
    /// Pipeline layout. `None` selects wgpu's auto-derive path.
    pub layout: Option<&'a PipelineLayout>,
    /// Vertex stage.
    pub vertex: VertexState<'a>,
    /// Fragment stage (optional — depth-only passes skip).
    pub fragment: Option<FragmentState<'a>>,
    /// Depth-stencil state (optional).
    pub depth_stencil: Option<DepthStencilState>,
}

/// Owned render pipeline.
#[derive(Clone, Debug)]
pub struct RenderPipeline {
    raw: Arc<wgpu::RenderPipeline>,
}

impl RenderPipeline {
    /// Compile a render pipeline.
    pub fn new(device: &Device, desc: &RenderPipelineDesc<'_>) -> Self {
        // Convert vertex attribute storage.
        let attr_storage: Vec<Vec<wgpu::VertexAttribute>> = desc
            .vertex
            .buffers
            .iter()
            .map(|b| {
                b.attributes
                    .iter()
                    .map(|a| wgpu::VertexAttribute {
                        offset: a.offset,
                        shader_location: a.shader_location,
                        format: a.format.to_wgpu(),
                    })
                    .collect()
            })
            .collect();
        let buffer_layouts: Vec<wgpu::VertexBufferLayout<'_>> = desc
            .vertex
            .buffers
            .iter()
            .enumerate()
            .map(|(i, b)| wgpu::VertexBufferLayout {
                array_stride: b.array_stride,
                step_mode: b.step_mode.to_wgpu(),
                attributes: &attr_storage[i],
            })
            .collect();

        let frag_targets: Option<Vec<Option<wgpu::ColorTargetState>>> =
            desc.fragment.as_ref().map(|f| {
                f.targets
                    .iter()
                    .map(|t| {
                        Some(wgpu::ColorTargetState {
                            format: t.format.to_wgpu(),
                            blend: None,
                            write_mask: wgpu::ColorWrites::ALL,
                        })
                    })
                    .collect()
            });

        let fragment = desc.fragment.as_ref().map(|f| wgpu::FragmentState {
            module: f.module.raw(),
            entry_point: Some(f.entry_point),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: frag_targets.as_deref().unwrap(),
        });

        let depth_stencil = desc.depth_stencil.map(|d| wgpu::DepthStencilState {
            format: d.format.to_wgpu(),
            depth_write_enabled: Some(d.depth_write_enabled),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        });

        let raw = device
            .raw()
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(desc.label),
                layout: desc.layout.map(|l| l.raw()),
                vertex: wgpu::VertexState {
                    module: desc.vertex.module.raw(),
                    entry_point: Some(desc.vertex.entry_point),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &buffer_layouts,
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil,
                multisample: wgpu::MultisampleState::default(),
                fragment,
                multiview_mask: None,
                cache: None,
            });
        Self { raw: Arc::new(raw) }
    }

    /// Query the per-set bind-group layout. When the pipeline was built
    /// with an auto-derived layout (`RenderPipelineDesc.layout = None`),
    /// this is the only way to retrieve the implicit layout that wgpu
    /// synthesised from shader reflection — bind-group construction
    /// keys against it.
    pub fn bind_group_layout(&self, set_index: u32) -> BindGroupLayout {
        BindGroupLayout {
            raw: Arc::new(self.raw.get_bind_group_layout(set_index)),
        }
    }

    /// Crate-internal access to the underlying `wgpu::RenderPipeline`.
    pub(crate) fn raw(&self) -> &wgpu::RenderPipeline {
        &self.raw
    }
}

/// Compute-pipeline descriptor.
///
/// `layout` is `Option<&PipelineLayout>` — see [`RenderPipelineDesc`]
/// for the auto-derive vs explicit-layout rationale (ADR-075 §8).
#[derive(Clone, Debug)]
pub struct ComputePipelineDesc<'a> {
    /// Debug label.
    pub label: &'a str,
    /// Pipeline layout. `None` selects wgpu's auto-derive path.
    pub layout: Option<&'a PipelineLayout>,
    /// Compute shader module.
    pub module: &'a ShaderModule,
    /// Entry point name (e.g. `"cs_main"`).
    pub entry_point: &'a str,
}

/// Owned compute pipeline.
#[derive(Clone, Debug)]
pub struct ComputePipeline {
    raw: Arc<wgpu::ComputePipeline>,
}

impl ComputePipeline {
    /// Compile a compute pipeline.
    pub fn new(device: &Device, desc: &ComputePipelineDesc<'_>) -> Self {
        let raw = device
            .raw()
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(desc.label),
                layout: desc.layout.map(|l| l.raw()),
                module: desc.module.raw(),
                entry_point: Some(desc.entry_point),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            });
        Self { raw: Arc::new(raw) }
    }

    /// Query the per-set bind-group layout. When the pipeline was built
    /// with an auto-derived layout (`ComputePipelineDesc.layout = None`),
    /// this is the only way to retrieve the implicit layout that wgpu
    /// synthesised from shader reflection — bind-group construction
    /// keys against it.
    pub fn bind_group_layout(&self, set_index: u32) -> BindGroupLayout {
        BindGroupLayout {
            raw: Arc::new(self.raw.get_bind_group_layout(set_index)),
        }
    }

    /// Crate-internal access to the underlying `wgpu::ComputePipeline`.
    pub(crate) fn raw(&self) -> &wgpu::ComputePipeline {
        &self.raw
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shader_stage_bitflags() {
        let s = ShaderStage::VERTEX | ShaderStage::FRAGMENT;
        assert!(s.contains(ShaderStage::VERTEX));
        assert!(s.contains(ShaderStage::FRAGMENT));
        assert!(!s.contains(ShaderStage::COMPUTE));
    }

    #[test]
    fn shader_stages_are_distinct() {
        assert!(!ShaderStage::VERTEX.contains(ShaderStage::FRAGMENT));
        assert!(!ShaderStage::VERTEX.contains(ShaderStage::COMPUTE));
        assert!(!ShaderStage::FRAGMENT.contains(ShaderStage::COMPUTE));
    }
}
