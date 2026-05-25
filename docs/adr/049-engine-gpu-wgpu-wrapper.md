# ADR-049 — `engine-gpu` owned wgpu wrapper crate

- Status: Accepted (Phase 5 design contract; new crate lands in Phase
  5 PR 2)
- Date: 2026-05-24
- Phase: 5 — RENDERING FOUNDATION (Track A)
- Companion: ADR-001 (Rust language), ADR-012 (50-year API
  contract), ADR-025 (audited crypto crates not owned — same
  precedent for vendoring well-scoped external libraries), ADR-037
  (Slang via slangc subprocess — same pattern), ADR-053 (Phase 5
  PR slicing)

## Context

Phase 5 needs a GPU abstraction. Three options are realistic in
2026:

1. Use `wgpu` directly across `engine-render`, `engine-physics`
   (GPU particles), `engine-audio` (some GPU compute), `engine-ai`
   (inference). Every layer imports `wgpu`.
2. Wrap `wgpu` in an owned Level-1 crate (`engine-gpu`) so that
   no other engine crate names `wgpu::*` types. wgpu becomes the
   implementation; `engine-gpu` becomes the contract.
3. Skip `wgpu` entirely; use `ash` (raw Vulkan) and write an owned
   abstraction.

Option 3 doubles Phase 5 scope (Vulkan synchronization, memory
allocation, descriptor heaps, swapchain, command-buffer recording —
months of work) and forfeits the web target (no WebGPU). The
Vulkan-spec exception in spec §I.1 line 91 doesn't extend to "and
also re-implement the entire Vulkan client library."

Option 1 is the fastest path but pins `engine-render`'s public API
to `wgpu`'s — and `wgpu`, like all GPU libraries, has had multiple
breaking releases (0.16, 0.17, 0.18, …). Pinning a 50-year API
contract (ADR-012) to a moving target is the same mistake the spec
warns about in §0.3 R-03.

Option 2 is the precedent we have: ADR-025 wraps audited crypto
crates (sha2, ed25519-dalek) into the asset pipeline without leaking
them to higher layers. ADR-037 wraps slangc into the engine-shader
toolchain as a sandboxed subprocess without leaking slangc's CLI
into render code. The same pattern applied to wgpu is the consistent
answer.

The user's planning decision in this session locks in option 2.
This ADR records it.

## Decision

### 1. New Level-1 crate `engine-gpu`

`crates/engine-gpu/`, added to the workspace `members` list. Depends
on:
- `wgpu` (direct, the only place in the workspace)
- `raw-window-handle` (transitively via wgpu; allowed because it's
  the standard Rust GPU-windowing trait crate)
- `engine-platform` (for `MmapAnon`, file-watch hot-reload, time)
- `engine-core` (for owned collections, telemetry signals)

No other engine crate may name `wgpu::*` types or import `wgpu`.

### 2. Public API — owned types only

```rust
// crates/engine-gpu/src/lib.rs (sketch)

pub struct Device { /* wraps wgpu::Device, wgpu::Queue */ }
pub struct Swapchain { /* wraps wgpu::Surface + texture rotation */ }
pub struct Buffer { /* wraps wgpu::Buffer */ }
pub struct Texture { /* wraps wgpu::Texture + wgpu::TextureView */ }
pub struct Sampler { /* wraps wgpu::Sampler */ }
pub struct CommandEncoder { /* wraps wgpu::CommandEncoder */ }
pub struct RenderPass<'a> { /* wraps wgpu::RenderPass */ }
pub struct ComputePass<'a> { /* wraps wgpu::ComputePass */ }
pub struct PipelineState { /* wraps wgpu::RenderPipeline / ComputePipeline */ }
pub struct BindlessHeap { /* per ADR-044, on top of wgpu::BindGroup */ }

pub enum DeviceLimits {
    Tier1Minimum,   // RX 580 milestone class
    Tier2Recommended,
    Tier3Enthusiast,
    Tier4AaaStudio,
}

impl Device {
    pub fn new(window: impl raw_window_handle::HasRawWindowHandle,
               limits: DeviceLimits) -> Result<Self, GpuError>;
    pub fn create_buffer(&self, desc: &BufferDesc) -> Buffer;
    pub fn create_texture(&self, desc: &TextureDesc) -> Texture;
    pub fn submit(&self, encoder: CommandEncoder) -> SubmitToken;
    // ... etc
}
```

Every parameter struct (`BufferDesc`, `TextureDesc`, …) is owned and
maps to wgpu's underlying type. Owned enums (`TextureFormat`,
`Usage`, `LoadOp`, `StoreOp`, …) mirror wgpu's where the semantics
align; new variants are added only when the engine needs something
wgpu exposes via a u32 flags struct.

### 3. CI grep guard — no `wgpu::` outside `crates/engine-gpu/`

A new step in `.github/workflows/ci.yml` gate job:

```yaml
- name: Guard against wgpu use outside engine-gpu (ADR-049)
  run: |
    hits=$(grep -rnE '\bwgpu::|use wgpu\b' crates bin tools testbed \
      | grep -v -e 'crates/engine-gpu/' \
      | grep -vE '^[^:]+:[0-9]+:[[:space:]]*//' \
      | grep -vE '^[^:]+:[0-9]+:[[:space:]]*!' \
      | grep -vE '\.md:' || true)
    if [ -n "$hits" ]; then
      echo "$hits"
      echo "::error::Route through engine_gpu instead of wgpu directly — see ADR-049"
      exit 1
    fi
```

Same shape as the ADR-028 / ADR-029 / ADR-032 guards. Comments
allowed, doc-comments allowed, .md files allowed.

### 4. Web target compatibility

`wgpu` is the only GPU library in 2026 that targets WebGPU + native
from one codebase. `engine-gpu` inherits this for free. ADR-006
(WGSL + WebTransport for web) flows naturally: web export builds
`engine-gpu` against `wgpu`'s WebGPU backend.

### 5. naga indirect dependency — still rejected directly

wgpu uses `naga` internally for WGSL parsing and shader-module
construction. naga arrives in the dependency tree as a transitive
of `wgpu`. ADR-037's grep guard rejects naga *direct* imports under
`tools/engine-shader/` — that guard stays exactly as it is. naga
inside wgpu's internals is not the engine's surface; it's
implementation.

This is the same pattern as `sha2`: ADR-025 permits sha2 in
`engine-asset`, but no higher crate imports sha2 directly because
they consume the asset layer's `ContentHash` type instead.

### 6. Version pinning

`wgpu = "0.20"` (or whatever the latest released version is at Phase
5 PR 2 land time; the PR pins it). Major version upgrades require
an ADR amendment because they may change the owned wrapper's
internals materially. Minor / patch versions land freely.

## Consequences

- One new crate; one new third-party dep added to the workspace
  Cargo.toml in Phase 5 PR 2. Both reviewable as one PR.
- The 50-year API contract (ADR-012) becomes defensible for GPU: if
  wgpu deprecates / forks / vanishes in 2030, `engine-gpu`'s
  public API doesn't change. The implementation file rewrites
  against `ash` or whatever succeeds wgpu.
- Performance neutral: wrapper types are zero-cost (transparent
  newtypes or direct field re-exposure); the wrapper does not
  introduce per-call overhead.
- Render-graph (ADR-039) and bindless heap (ADR-044) sit on top of
  `engine-gpu`. Material system, mesh upload, texture upload all
  call `engine_gpu::Device` methods, never `wgpu`.

## Risks and tradeoffs

- **Owned wrapper is real code to maintain.** ~1 500 LOC estimate
  for the Phase 5 surface. Acceptable — the alternative is binding
  to wgpu's surface, which is also ~1 500 LOC of mental tax across
  upper crates.
- **wgpu's bleeding-edge features land in `engine-gpu` only after
  a wrapping PR.** This is a deliberate friction: it prevents
  speculative dependency on wgpu features that might not stabilize.
  When a feature is wrap-worthy, an ADR amendment lands it.
- **Web export needs the wrapper's web variant.** Same code path
  internally (wgpu's WebGPU backend); the wrapper's public API
  doesn't change. Documented in `engine-gpu`'s architecture doc.
- **`raw-window-handle` is a small extra dep.** Used only at
  `Device::new` time; transitively required by wgpu anyway. Not a
  policy issue.
- **Compile time** grows by wgpu's compile time (notable —
  ~30 s cold). Mitigated by sccache; reviewers see it on a clean
  build only.

## Alternatives considered

- **Direct wgpu in every render-touching crate.** Convenient now,
  costly later. Rejected: violates ADR-012 spirit.
- **Direct `ash` (Vulkan) without wgpu.** Triples Phase 5 scope.
  Forfeits web. Rejected: too expensive for the ownership win.
- **`vulkano` (high-level Rust Vulkan).** Less mature than wgpu;
  no WebGPU path. Rejected.
- **Defer the wrapper until Phase 9 (when the wgpu API has settled
  even more).** Tempting but punts the abstraction debt to a phase
  that has its own large surface. Rejected: now-is-cheap, later-is-
  expensive.

## Verification

- Lands in Phase 5 PR 2. The PR's tests:
  - `tests/gpu_device_init.rs`: `Device::new` succeeds with each of
    the four tier limits on a CI runner with an actual GPU.
  - `tests/gpu_buffer_roundtrip.rs`: write bytes, read bytes back;
    same bytes.
  - `tests/gpu_swapchain_resize.rs`: resize the swapchain; verify no
    panic and the new dimensions take effect.
  - `tests/gpu_bindless_heap.rs`: see ADR-044's tests; the bindless
    heap is the first non-trivial wrapper consumer.
- CI: the new grep guard runs on every PR (gate job; not gated to
  render-touching paths, since the goal is to keep new code from
  reaching past the boundary anywhere).
- Architecture doc: `docs/architecture/engine-gpu.md` (lands with
  the PR; the only new architecture doc Phase 5 produces).
- Telemetry: GPU subsystem signals will be defined in `engine-gpu`
  and consumed by the renderer (`SPAN("gpu.submit",
  Subsystem::Render)`, `GAUGE "gpu.vram_used_bytes"`, etc.).
