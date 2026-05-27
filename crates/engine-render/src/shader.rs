//! Slang artefact → GPU pipeline binding (ADR-063).
//!
//! Phase 4 PR 4 (ADR-037) shipped the Slang toolchain that produces
//! [`engine_shader::Bundle`] artefacts — per-target compiled bytes
//! (SPIR-V / WGSL / DXIL / MSL) plus reflection JSON plus a BLAKE3
//! digest. Phase 5 PR 2 (ADR-049) shipped `engine_gpu::ShaderModule`
//! and the [`engine_gpu::RenderPipeline`] / [`engine_gpu::ComputePipeline`]
//! constructors. This module is the connecting layer: it consumes a
//! shader [`Bundle`] and an `engine_gpu::Device` and produces a
//! ready-to-bind pipeline, without leaking `wgpu::*` identifiers past
//! the ADR-049 boundary.
//!
//! ## Why a thin layer
//!
//! Every Phase-6 GPU pass needs the same plumbing: pick the artefact
//! that matches the device's backend, build a shader module from its
//! bytes, build a pipeline-layout, compose a pipeline-state-object.
//! Without this layer each pass would re-implement the selection
//! cascade; with it the convention is one place.
//!
//! ## Out of scope (deferred to PR 3+)
//!
//! - Reflection-driven `BindGroupLayout` synthesis. PR 2 ships an
//!   empty layout helper that suits trivial demo shaders; PR 3
//!   geometry passes will populate the layout from the artefact's
//!   reflection JSON or — for the bindless-heavy passes — from
//!   hard-coded layouts that mirror ADR-064's contracts.
//! - The shader-pipeline cache (`PipelineCache`). PR 2 builds
//!   pipelines on demand; the per-frame consumer count is small
//!   enough that caching matters first when the geometry passes
//!   construct ~10 pipelines per frame in PR 3.

use engine_gpu::{
    BindGroupLayoutDesc, ComputePipeline, ComputePipelineDesc, Device, PipelineLayout,
    PipelineLayoutDesc, RenderPipeline, RenderPipelineDesc, ShaderModule, ShaderModuleDesc,
    VertexState,
};
use engine_shader::{Artifact, Bundle, Stage, Target};

/// Why a shader artefact can't be turned into a GPU pipeline.
#[derive(Debug)]
pub enum ShaderError {
    /// The bundle does not contain an artefact for the device's
    /// preferred backend. Typically means the bundle was compiled
    /// against a target subset that doesn't include WGSL (the
    /// canonical `engine_gpu` consumer today).
    TargetNotInBundle(Target),
    /// The artefact's bytes are not valid UTF-8 — relevant only for
    /// WGSL/MSL targets (binary targets are bytes-as-bytes).
    NotUtf8 {
        /// Which target the decode failed on.
        target: Target,
    },
}

impl core::fmt::Display for ShaderError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ShaderError::TargetNotInBundle(t) => {
                write!(f, "shader bundle does not contain target {t:?}")
            }
            ShaderError::NotUtf8 { target } => {
                write!(f, "shader bytes for target {target:?} are not valid UTF-8")
            }
        }
    }
}

impl std::error::Error for ShaderError {}

/// A handle to a compiled shader [`Bundle`] that the render graph can
/// resolve into per-device GPU resources.
///
/// Wraps the `engine_shader` artefact set without leaking its types
/// further than necessary — the renderer constructs pipelines by
/// going through helpers in this module rather than reaching into
/// the bundle itself.
#[derive(Clone, Debug)]
pub struct ShaderArtefactSet {
    bundle: Bundle,
}

impl ShaderArtefactSet {
    /// Wrap a bundle.
    pub fn new(bundle: Bundle) -> Self {
        Self { bundle }
    }

    /// Read access to the underlying bundle (digest, entry-point name,
    /// per-target artefacts).
    pub fn bundle(&self) -> &Bundle {
        &self.bundle
    }

    /// Pick the WGSL artefact. `engine_gpu` is wgpu-backed today, so
    /// WGSL is the only target the runtime consumes; SPIR-V / DXIL /
    /// MSL ride along in the bundle for future native-backend work.
    pub fn wgsl(&self) -> Result<&Artifact, ShaderError> {
        self.bundle
            .artifacts
            .iter()
            .find(|a| a.target == Target::Wgsl)
            .ok_or(ShaderError::TargetNotInBundle(Target::Wgsl))
    }

    /// Pick the artefact that matches the device's preferred backend.
    /// Today this always means WGSL — when PR 6+ introduces native
    /// Vulkan / D3D12 / Metal backends, this widens to consult
    /// `Device::backend()`.
    pub fn for_device(&self, _device: &Device) -> Result<&Artifact, ShaderError> {
        self.wgsl()
    }
}

/// Render-pipeline helper descriptor. Mirrors `engine_gpu::RenderPipelineDesc`
/// but takes [`ShaderArtefactSet`]s instead of pre-built
/// [`ShaderModule`]s — the helper builds the modules from the artefact
/// bytes.
#[derive(Clone, Debug)]
pub struct RenderPipelineHelperDesc<'a> {
    /// Debug label.
    pub label: &'a str,
    /// Vertex shader artefact set.
    pub vertex: &'a ShaderArtefactSet,
    /// Vertex entry point name (e.g. `"vs_main"`).
    pub vertex_entry: &'a str,
    /// Vertex buffer layouts.
    pub vertex_buffers: &'a [engine_gpu::VertexBufferLayout<'a>],
    /// Fragment shader artefact set (optional — depth-only passes
    /// skip).
    pub fragment: Option<&'a ShaderArtefactSet>,
    /// Fragment entry point name (e.g. `"fs_main"`).
    pub fragment_entry: &'a str,
    /// Colour targets.
    pub color_targets: &'a [engine_gpu::ColorTargetState],
    /// Depth-stencil state.
    pub depth_stencil: Option<engine_gpu::DepthStencilState>,
}

/// Compute-pipeline helper descriptor.
#[derive(Clone, Debug)]
pub struct ComputePipelineHelperDesc<'a> {
    /// Debug label.
    pub label: &'a str,
    /// Compute shader artefact set.
    pub compute: &'a ShaderArtefactSet,
    /// Compute entry point name (e.g. `"cs_main"`).
    pub entry: &'a str,
}

/// Build a `RenderPipeline` from a [`ShaderArtefactSet`] pair.
///
/// The helper picks the WGSL artefact for each stage, decodes it as
/// UTF-8 source, builds an `engine_gpu::ShaderModule`, constructs a
/// minimal `PipelineLayout` (empty bind groups — PR 3 widens this when
/// the geometry passes need group-0/1/2/3 bindings per ADR-063 §4),
/// and assembles the pipeline.
pub fn build_render_pipeline(
    device: &Device,
    desc: &RenderPipelineHelperDesc<'_>,
) -> Result<RenderPipeline, ShaderError> {
    let vertex_module = build_shader_module(device, desc.vertex, desc.label, "vertex")?;
    let fragment_module = match desc.fragment {
        Some(fs) => Some(build_shader_module(device, fs, desc.label, "fragment")?),
        None => None,
    };

    let layout = PipelineLayout::new(
        device,
        &PipelineLayoutDesc {
            label: desc.label,
            bind_group_layouts: &[],
        },
    );

    let fragment_state = fragment_module
        .as_ref()
        .map(|module| engine_gpu::FragmentState {
            module,
            entry_point: desc.fragment_entry,
            targets: desc.color_targets,
        });

    let pipeline_desc = RenderPipelineDesc {
        label: desc.label,
        layout: &layout,
        vertex: VertexState {
            module: &vertex_module,
            entry_point: desc.vertex_entry,
            buffers: desc.vertex_buffers,
        },
        fragment: fragment_state,
        depth_stencil: desc.depth_stencil,
    };
    Ok(RenderPipeline::new(device, &pipeline_desc))
}

/// Build a `ComputePipeline` from a [`ShaderArtefactSet`].
pub fn build_compute_pipeline(
    device: &Device,
    desc: &ComputePipelineHelperDesc<'_>,
) -> Result<ComputePipeline, ShaderError> {
    let module = build_shader_module(device, desc.compute, desc.label, "compute")?;
    let layout = PipelineLayout::new(
        device,
        &PipelineLayoutDesc {
            label: desc.label,
            bind_group_layouts: &[],
        },
    );
    let pipeline_desc = ComputePipelineDesc {
        label: desc.label,
        layout: &layout,
        module: &module,
        entry_point: desc.entry,
    };
    Ok(ComputePipeline::new(device, &pipeline_desc))
}

/// Build an empty bind-group layout. PR 3 widens this when the
/// geometry passes need real bind-group entries; PR 2's helpers ride
/// on the empty layout so demo / smoke pipelines compile without
/// requiring a reflection-driven layout extractor.
pub fn empty_bind_group_layout(device: &Device, label: &str) -> engine_gpu::BindGroupLayout {
    engine_gpu::BindGroupLayout::new(device, &BindGroupLayoutDesc { label })
}

/// Wrap a raw WGSL `&str` (e.g. one of the embedded `crate::shaders::*`
/// constants) into a [`ShaderArtefactSet`] so the `build_*_pipeline`
/// helpers can consume it without a `slangc` build step.
///
/// The hand-written Phase-6 WGSL sources at
/// `crates/engine-render/shaders/*.wgsl` ship as `pub const &str`
/// constants embedded via `include_str!`. They never went through
/// `slangc`, so they don't ride in a multi-target Slang
/// [`Bundle`]. This helper builds a one-artefact bundle so those
/// strings flow through the same pipeline-builder API.
pub fn wgsl_artefact_set(stage: Stage, entry: &str, source: &str) -> ShaderArtefactSet {
    let artefact = Artifact::new(Target::Wgsl, source.as_bytes().to_vec(), Vec::new());
    let bundle = Bundle::new(entry, stage, vec![artefact]);
    ShaderArtefactSet::new(bundle)
}

fn build_shader_module(
    device: &Device,
    artefacts: &ShaderArtefactSet,
    label: &str,
    stage_label: &str,
) -> Result<ShaderModule, ShaderError> {
    let artefact = artefacts.for_device(device)?;
    let wgsl = core::str::from_utf8(&artefact.bytes).map_err(|_| ShaderError::NotUtf8 {
        target: artefact.target,
    })?;
    let module_label = format!("{label}:{stage_label}");
    Ok(ShaderModule::new(
        device,
        &ShaderModuleDesc {
            label: &module_label,
            wgsl,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_shader::{Artifact, Bundle, Stage, Target};

    fn make_bundle() -> Bundle {
        Bundle::new(
            "vs_main",
            Stage::Vertex,
            vec![
                Artifact::new(
                    Target::SpirV,
                    vec![0xde, 0xad, 0xbe, 0xef],
                    Vec::new(),
                ),
                Artifact::new(
                    Target::Wgsl,
                    b"@vertex fn vs_main() -> @builtin(position) vec4<f32> { return vec4<f32>(0); }"
                        .to_vec(),
                    Vec::new(),
                ),
            ],
        )
    }

    #[test]
    fn wgsl_artefact_is_selected() {
        let set = ShaderArtefactSet::new(make_bundle());
        let a = set.wgsl().expect("wgsl present");
        assert_eq!(a.target, Target::Wgsl);
        assert!(a.bytes.starts_with(b"@vertex"));
    }

    #[test]
    fn missing_target_errors() {
        let bundle = Bundle::new(
            "cs_main",
            Stage::Compute,
            vec![Artifact::new(Target::SpirV, vec![0x07, 0x23], Vec::new())],
        );
        let set = ShaderArtefactSet::new(bundle);
        assert!(matches!(
            set.wgsl(),
            Err(ShaderError::TargetNotInBundle(Target::Wgsl))
        ));
    }

    #[test]
    fn artefact_set_preserves_bundle_metadata() {
        let bundle = make_bundle();
        let entry = bundle.entry.clone();
        let stage = bundle.stage;
        let set = ShaderArtefactSet::new(bundle);
        assert_eq!(set.bundle().entry, entry);
        assert_eq!(set.bundle().stage, stage);
    }

    #[test]
    fn shader_error_displays() {
        let err = ShaderError::TargetNotInBundle(Target::Wgsl);
        let s = err.to_string();
        assert!(s.contains("Wgsl"), "{s}");
    }

    #[test]
    fn wgsl_artefact_set_wraps_vertex_source() {
        let src = "@vertex fn vs_main() -> @builtin(position) vec4<f32> { return vec4<f32>(0); }";
        let set = wgsl_artefact_set(Stage::Vertex, "vs_main", src);
        assert_eq!(set.bundle().entry, "vs_main");
        assert_eq!(set.bundle().stage, Stage::Vertex);
        let a = set.wgsl().expect("wgsl present");
        assert_eq!(a.target, Target::Wgsl);
        assert_eq!(a.bytes, src.as_bytes());
    }

    #[test]
    fn wgsl_artefact_set_wraps_compute_source() {
        let src = "@compute @workgroup_size(1) fn cs_main() {}";
        let set = wgsl_artefact_set(Stage::Compute, "cs_main", src);
        assert_eq!(set.bundle().stage, Stage::Compute);
        let a = set.wgsl().expect("wgsl present");
        assert!(a.bytes.starts_with(b"@compute"));
    }
}
