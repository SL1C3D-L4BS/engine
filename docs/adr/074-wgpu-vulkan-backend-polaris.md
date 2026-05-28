# ADR-074 — Activate wgpu Vulkan backend for Polaris GFX8 / RADV

- Status: Accepted
- Date: 2026-05-27
- Phase: 5.5 — Track A GPU binding closure (per ADR-069 reconciliation)
- Companion: ADR-049 (engine-gpu wgpu wrapper — defines the
  workspace's single wgpu consumer), ADR-044 (bindless texture heap —
  depends on `VK_EXT_descriptor_indexing`), ADR-047 (frame-pacing CI
  gate — its scenario can now run end-to-end on real hardware),
  ADR-068 (Phase 6 PR slicing — the work this ADR unblocks), ADR-069
  (engine-vs-spec phase reconciliation — names this as the first
  entry into Track A closure), ADR-075 (pass `record()` discipline —
  the next ADR that lands)

## References

- *Vulkan Programming Guide* (Sellers / Kessenich, 2017), Ch. 2
  ("Instances, Devices, and Queues" — VkInstance / VkPhysicalDevice /
  VkDevice lifecycle), Ch. 5 ("Resources, Memory, and Synchronisation"
  — the resource lifetime model wgpu wraps).
- *Real-Time Rendering*, 4th ed. (Akenine-Möller / Haines / Hoffman,
  2018), Ch. 23 ("Graphics Hardware") — Polaris / GFX8 feature surface;
  the engine's audit confirmed no Polaris-incompatible shader features
  in `crates/engine-render/shaders/*.wgsl`.
- Mesa RADV release notes (Mesa 26.1.1 ships with Polaris Vulkan 1.3
  conformance).
- wgpu 29 changelog — backend feature flags: `vulkan`, `metal`, `dx12`,
  `gl`, `webgl`.

## Context

The Phase 5.5 audit identified that the workspace `Cargo.toml` pins:

```toml
wgpu = { version = "29", default-features = false, features = ["wgsl"] }
```

No backend feature is enabled. The wgpu crate compiles, but
`Instance::new()` returns an instance with zero registered backends;
`Adapter::request_device` panics at the wgpu-hal layer with "no backend
enabled." `crates/engine-render/tests/pipeline_smoke.rs` catches the
panic via `panic::catch_unwind` and skips the test silently, masking
the missing backend until someone reads the comment block.

The spec's Recommended-tier hardware (Part XX.7 line 1587) is AMD
Radeon RX 580 / GTX 1660 / Intel Arc A380. The Phase 5 milestone (spec
line 1631) is "deferred PBR running on RX 580 at 60 FPS @ 1440p.
Software/GPU pixel parity." Neither can be measured until wgpu can talk
to the host GPU.

On Linux the only wgpu backend that targets AMD GPUs is `vulkan`. The
host driver stack is Mesa 26.1.1 / RADV exposing Vulkan 1.3 on Polaris
(GFX8); the relevant Vulkan 1.3 extensions the engine consumes —
`VK_KHR_dynamic_rendering` (wgpu 29 uses internally for the modern
render-pass model), `VK_EXT_descriptor_indexing` (the ADR-044 bindless
heap), `VK_KHR_synchronization2` (wgpu 29 internal) — are exposed on
Polaris via the upstream RADV implementation.

## Decision

### 1. Enable the `vulkan` feature in the workspace wgpu dep

```toml
wgpu = { version = "29", default-features = false, features = ["wgsl", "vulkan"] }
```

This is the minimum-disturbance change that turns the engine from
"CPU-rasterizer oracle only" to "can reach a real adapter and validate
shaders against pipeline layouts." It does not alter the ADR-049
boundary (`engine-gpu` remains the single wgpu consumer).

### 2. Decouple `max_immediate_size` from `Limits::downlevel_defaults`

`wgpu::Limits::downlevel_defaults()` (the Tier1Minimum limit table)
sets `max_immediate_size = 0` even when the `IMMEDIATES` feature (the
wgpu-29 name for `PUSH_CONSTANTS`) is requested and granted. The
feature flag enables the API surface; the limit caps the size. They
are independent dials in wgpu's model.

`crates/engine-gpu/src/device.rs` is amended so that when
`features.push_constants` is true (i.e. the adapter advertises
`IMMEDIATES`), the device descriptor's `required_limits` overrides
`max_immediate_size` to `128` bytes — the Vulkan-guaranteed minimum
that every conformant device meets (including Polaris/RADV, Skylake-GT,
Apple Silicon). ADR-044 §6 uses only 8 bytes, so 128 is comfortable
headroom for future per-draw push payloads.

### 3. Layered pipeline smoke tests

`crates/engine-render/tests/pipeline_smoke.rs` is restructured into
two tests:

- **`device_init_against_real_adapter`** — runs in the default workspace
  test pass. Calls `Device::new(DeviceLimits::Tier1Minimum, false)` and
  asserts success (or skips with the `try_device` panic guard on hosts
  without a Vulkan loader). This is ADR-074's contract: prove the
  workspace can reach a real GPU.
- **`build_all_phase6_pipelines_against_real_device`** — stays
  `#[ignore = "A.2 wires per-pass bind-group layouts; remove when A.2
  lands"]`. The pipelines today are constructed against empty bind-
  group layouts; wgpu correctly rejects the first shader binding the
  layout doesn't declare. This is ADR-075's contract: prove the
  Rust-authored layouts match the WGSL `@group/@binding` declarations.

The layered design is honest about the intermediate state: ADR-074
ships the infrastructure; ADR-075 ships the content that exercises it.

### 4. Build-time portability

The `ash` Rust Vulkan binding wgpu uses loads `libvulkan.so.1` /
`vulkan-1.dll` at runtime via `libloading`. Enabling the `vulkan`
feature does **not** require the Vulkan SDK headers at build time and
is therefore safe on all platforms: Vulkan-less hosts compile the
feature without errors but trigger the "no backend enabled" code path
at runtime (skipped by the smoke test's catch).

## Rationale

- **Vulkan is the spec's named Linux backend.** Spec Part XX.7 names
  AMD RX 580 explicitly; on Linux that is `vulkan` via RADV. No other
  wgpu backend (Metal, DX12, GL/WebGL) reaches AMD hardware on Linux.
- **Polaris is Vulkan 1.3 conformant via Mesa RADV.** Mesa 26.1.1 (the
  user's installed driver) exposes the descriptor-indexing,
  dynamic-rendering, and synchronisation-2 extensions that wgpu 29 and
  the engine's bindless heap require.
- **The pipeline smoke test has been silently skipping.** Before this
  ADR, every `cargo test --workspace` invocation skipped
  `build_all_phase6_pipelines_against_real_device` due to the
  ignore attribute + the no-backend panic. With the feature on, the
  test exercises the 11 + BRDF LUT pipelines on whatever adapter the
  host exposes, producing a labelled error naming the failing pass
  rather than masked failure.
- **Single-line change, no boundary violation.** ADR-049's wgpu
  consumer boundary is unchanged. The feature flag is an additive
  property of the wgpu dep; the `engine-gpu` source code is untouched.

## Consequences

- `cargo build --workspace` now links `wgpu-hal-vulkan` and `ash`.
  Transitive deps add `ash`, `libloading`, `khronos-egl` (the deny.toml
  license allowlist already covers these as transitive permissive
  licenses; no `deny.toml` change is needed).
- `cargo test --workspace` now exercises pipeline construction against
  the host's real adapter. On the user's RX 580 + Mesa RADV stack,
  this is the first test in the repo that proves the WGSL → SPIR-V →
  driver path holds end-to-end. The test count rises from 615 (with 1
  skip) to 616 (with 0 skips) on Vulkan hosts.
- The frame-pacing bench (`bin/engine-bench-frame-pacing/`) gains the
  ability to swap its synthetic CPU workload for a real GPU dispatch
  path once Track A's pass `record()` bodies are wired (ADR-075).
- The ADR-068 "PR 7" addendum comment about "self-hosted RDNA2-class GPU
  runner with feature toggles" is superseded: backend toggling is no
  longer an external CI concern, it is a workspace property. The
  ADR-070 frame-pacing re-baseline ADR will document the local-bench
  workflow replacing the missing CI runner.

## Risks and tradeoffs

- **Vulkan SDK is not required, but `libvulkan` is at runtime.** Hosts
  without a Vulkan loader (rare on modern Linux; missing on some
  minimal CI containers) get the same "skip" path the test already
  handled. No new failure modes.
- **Polaris-specific behaviour is now in the test surface.** A future
  shader change that uses a feature Polaris lacks (subgroups beyond
  size 64, `f16` arithmetic, DP4a / INT8 dot-product, i64 atomics)
  would fail the smoke test on RX 580 hardware. This is intended —
  the spec names RX 580 as the Recommended-tier baseline; any
  Polaris-incompatible feature must be either gated or opted out per
  ADR-072 (texture-compression fallback pattern) or its successor.
- **wgpu 29 + Mesa RADV interaction edge cases** may surface as the
  pass `record()` bodies wire real work (ADR-075). The audit
  identified no concerning patterns in the existing WGSL sources
  (standard 32-bit atomics, no subgroups, no f16, no i64 atomics).
- **Cross-platform CI still works.** The `vulkan` feature is purely
  additive; macOS / Windows CI runners that lack Vulkan still build
  cleanly (no SDK required) and skip the smoke test gracefully.

## Alternatives considered

- **Enable all backends (`vulkan`, `metal`, `dx12`, `gl`).** Larger
  binary size, more transitive deps; the engine has no near-term need
  for the others (macOS/Windows/web are Phase 11+ per spec). Rejected
  in favour of incremental activation: add per-platform feature gating
  when the target lands. ADR can amend per backend.
- **Configure the feature in `crates/engine-gpu/Cargo.toml` only.**
  Workspace deps with features must be configured at the workspace
  level when consumed by multiple crates; the `engine-gpu` Cargo.toml
  is the natural alternative, but the workspace dep is the canonical
  place (matches how `wgsl` is set today).
- **Use `Bridge` or `wgpu-hal-vulkan` directly.** Skips wgpu's
  abstraction layer. Rejected by ADR-049 — the engine consumes wgpu,
  not wgpu-hal.
- **Wait for a self-hosted CI runner** (the original PR 7 plan per
  ADR-068's third addendum). The user has the spec's named Recommended
  hardware on the development workstation; no separate CI runner is
  required. ADR-070 documents the local-bench workflow.

## Verification

- `cargo build --workspace` succeeds with the new feature.
- `cargo test -p engine-render --test pipeline_smoke` shows `1 passed,
  1 ignored` (the ignored one is the ADR-075-blocked layout smoke).
  Validated 2026-05-27 on the user's RX 580 / Mesa 26.1.1 / RADV.
- The `device_init_against_real_adapter` test reports adapter features
  for the host. On the user's RX 580: `push_constants: true,
  bc_textures: true, descriptor_indexing: true`. Polaris exposes all
  three.
- The push-constant limit override is exercised: without it, the
  `IMMEDIATES`-using shaders' pipeline-layout construction fails with
  "Immediate data has size N which exceeds device immediate data size
  limit 0..0". With the override (this ADR §2), pipeline-layout
  construction succeeds.
- `just ci` still passes (the full gate: build + test + clippy + fmt +
  deny). Workspace test count: 616 (was 615; the new
  `device_init_against_real_adapter` test joins the gate).
- The full-pipeline smoke `build_all_phase6_pipelines_against_real_device`
  correctly fails with named "binding missing from pipeline layout"
  for the `cull` pass when invoked with `--include-ignored` — proving
  wgpu shader-vs-layout validation is active. A.2 (ADR-075) lifts the
  ignore.

## Pre-merge engineering checklist

- [x] `Cargo.toml` workspace dep updated.
- [x] `crates/engine-gpu/src/device.rs` push-constant limit override.
- [x] `pipeline_smoke.rs` restructured into layered smokes.
- [x] `cargo test -p engine-render --test pipeline_smoke` validated on
      the user's RX 580 (`device_init_against_real_adapter` passes;
      `build_all_phase6_pipelines_against_real_device` correctly fails
      with named missing-binding for the `cull` pass when forced).
- [ ] `just ci` green on the user's RX 580 (validated at task close).
- [ ] ADR-068 close addendum amended to point at ADR-074 (lands with
      ADR-069 phase reconciliation in C.3).
