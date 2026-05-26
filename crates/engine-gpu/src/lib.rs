//! `engine-gpu` — owned wgpu wrapper (Level 1).
//!
//! See `ENGINE_SPECIFICATION_v2.0.md` Part IV.1 + Part IV.4.A and ADR-049
//! (`docs/adr/049-engine-gpu-wgpu-wrapper.md`).
//!
//! Single permissible consumer of [`wgpu`] in the workspace. Every higher
//! crate names types from this crate; the `wgpu::` boundary CI grep guard
//! in `.github/workflows/ci.yml` enforces the rule by rejecting `wgpu::`
//! identifiers anywhere outside `crates/engine-gpu/`.
//!
//! # Module map
//!
//! - [`error`] — owned [`GpuError`] enum surfaced from every fallible call.
//! - [`device`] — [`Device`], [`Queue`], [`DeviceLimits`]. The `Device::new`
//!   constructor is the wgpu-init entry point.
//! - [`buffer`] — [`Buffer`] + [`BufferDesc`] + [`BufferUsage`].
//! - [`texture`] — [`Texture`], [`TextureView`], [`TextureDesc`],
//!   [`TextureFormat`] (covering the BC{4,5,6,7} codecs per ADR-045).
//! - [`sampler`] — [`Sampler`] + [`SamplerDesc`] + filter / address enums.
//! - [`swapchain`] — [`Swapchain`] + [`SwapchainConfig`] + [`PresentMode`].
//! - [`encoder`] — [`CommandEncoder`], [`RenderPass`], [`ComputePass`],
//!   [`SubmitToken`].
//! - [`pipeline`] — [`ShaderModule`], [`BindGroupLayout`], [`PipelineLayout`],
//!   [`RenderPipeline`], [`ComputePipeline`].
//! - [`bindless`] — [`BindlessHeap`], [`BindlessTextureId`],
//!   [`BindlessSamplerId`] per ADR-044. Pure-Rust slot accounting on top
//!   of the wrapper types; no GPU is required to exercise it.
//!
//! # Design contracts
//!
//! - **Owned API surface.** Every public type is owned: no [`wgpu`] types
//!   leak across the crate boundary. New variants on owned enums (e.g.
//!   [`TextureFormat`]) are added only when the engine actually needs them;
//!   wgpu's full enum is intentionally not mirrored.
//! - **Zero-cost wrappers.** Every wrapper is either a transparent newtype
//!   or a struct whose only field is the corresponding wgpu handle. No
//!   per-call overhead is introduced.
//! - **Synchronous façade.** wgpu's adapter / device futures are polled to
//!   completion via a tiny in-crate `block_on` (no executor dependency);
//!   the public API is sync, matching the rest of the engine.
//! - **Bindless first-class.** [`BindlessHeap`] is the only place
//!   descriptor-set bookkeeping lives. PR 3+ render passes never touch
//!   per-material bind groups; per-draw push constants carry the
//!   [`BindlessTextureId`] + [`BindlessSamplerId`].

pub mod bindless;
pub mod buffer;
pub mod device;
pub mod encoder;
pub mod error;
pub mod pipeline;
pub mod sampler;
pub mod swapchain;
pub mod texture;

mod runtime;

pub use bindless::{
    BindlessHeap, BindlessHeapConfig, BindlessHeapStats, BindlessSamplerId, BindlessTextureId,
    FALLBACK_TEXTURE_SLOT, HeapFull, MAGENTA_FALLBACK_GENERATION,
};
pub use buffer::{Buffer, BufferDesc, BufferUsage};
pub use device::{Device, DeviceFeatures, DeviceLimits, Queue};
pub use encoder::{CommandEncoder, ComputePass, RenderPass, SubmitToken};
pub use error::GpuError;
pub use pipeline::{
    BindGroupLayout, BindGroupLayoutDesc, ColorTargetState, ComputePipeline, ComputePipelineDesc,
    DepthStencilState, FragmentState, PipelineLayout, PipelineLayoutDesc, RenderPipeline,
    RenderPipelineDesc, ShaderModule, ShaderModuleDesc, ShaderStage, VertexAttribute,
    VertexBufferLayout, VertexFormat, VertexState, VertexStepMode,
};
pub use sampler::{AddressMode, FilterMode, Sampler, SamplerDesc};
pub use swapchain::{PresentMode, Swapchain, SwapchainConfig, SwapchainTexture};
pub use texture::{
    Extent3d, Texture, TextureDesc, TextureDimension, TextureFormat, TextureUsage, TextureView,
};
