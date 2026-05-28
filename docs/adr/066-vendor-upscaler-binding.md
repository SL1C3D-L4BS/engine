# ADR-066 — Vendor upscaler binding discipline

- Status: Accepted (Phase 6 design contract; implementation lands in
  Phase 6 PR 5)
- Date: 2026-05-27
- Phase: 6 — RENDERING FOUNDATION (Track A, Part 2)
- Companion: ADR-005 (vendor upscaler first, owned fallback —
  parent contract), ADR-019 (asset sandbox subprocesses — pattern
  for the SDK runtime call), ADR-025 (audited crypto crates not
  owned — same vendoring discipline), ADR-049 (engine-gpu wrapper),
  ADR-051 (acknowledged deviations register — vendor binary deps
  log here), ADR-067 (Owned::OnnxTemporal — sibling upscaler),
  ADR-068 (Phase 6 PR slicing)

## Context

Phase 5 PR 5 (commit `5422277`) shipped the `UpscalerProvider` trait
surface in `crates/engine-render/src/upscale.rs` with three vendor
stubs (`VendorDlss`, `VendorFsr`, `VendorXess`) all returning
`supports() == false` and one `OwnedBilinear` placeholder. ADR-005's
Consequences are explicit: *"Phase 6 expands each vendor stub to a
real binding."*

Vendor upscaler SDKs in 2026:

- **NVIDIA DLSS** via Streamline 2.x. Closed-source loader (.dll /
  .so), runtime feature query, opaque tensor-core path on RTX 20+.
- **AMD FSR 4** via the AMD FidelityFX SDK. Open-source but vendored
  per-version (FSR 4 ships with the AMD GPUOpen distribution); RDNA
  4 tensor path + RDNA 3 / others FSR 3.x spatial fallback.
- **Intel XeSS 2** via the Intel XeSS SDK. DP4a path on cross-vendor
  GPUs, tensor path on Arc.

All three SDKs have non-crates.io distribution (download + license
acceptance from the vendor), runtime binary blobs (`.dll`, `.so`,
`.dylib`), and version cadences independent of the engine's release
cycle. They are also the *highest-quality* upscalers on their target
hardware; ADR-005 commits the engine to consuming them where present.

ADR-019's subprocess sandbox addresses the security half of "should
the vendor SDK live inside the editor's address space" — but
upscalers are not asset importers. They run *every frame*, consume
GPU memory the renderer has already allocated, and integrate via
device-shared interop handles. The subprocess pattern would force
~16 ms of per-frame IPC overhead — infeasible at 60 FPS.

The decision space is therefore: how to integrate vendor SDKs into
the runtime *without* relaxing the owned-discipline that ADR-049
established for `wgpu` and ADR-037 established for `slangc`.

## Decision

### 1. New crate `engine-upscale-vendor` (Level 1)

`crates/engine-upscale-vendor/`, added to the workspace `members`
list. Depends on:

- `engine-gpu` (consumes `engine_gpu::Device`, `Texture`, etc.)
- `engine-render` (publishes the `UpscalerProvider` impls)
- Optionally per cargo feature: `streamline-sys` (DLSS),
  `fsr-sys` (FSR 4), `xess-sys` (XeSS)

The `*-sys` crates are NOT placed in crates.io. They live in
`tools/upscaler-vendor-sdks/{streamline,fsr,xess}/`, vendored as
git-tracked Cargo paths. Each `*-sys` crate ships:

- A minimal Rust FFI binding (`bindgen`-generated from the
  vendor's C header, committed to the repo so build doesn't depend
  on `bindgen`).
- A `build.rs` that links the vendor's shared library (`.so` /
  `.dll`).
- A `LICENSE-VENDOR.txt` file mirroring the SDK's license.

### 2. Cargo features cascade

```toml
# crates/engine-upscale-vendor/Cargo.toml
[features]
default = []
dlss = ["streamline-sys"]
fsr   = ["fsr-sys"]
xess  = ["xess-sys"]
all-vendors = ["dlss", "fsr", "xess"]
```

- **Default build (no features)** ships the engine with vendor
  stubs that mirror Phase 5's `supports() == false` behavior. CI
  (GitHub-hosted, no SDKs) builds and tests the engine in this
  configuration without ever touching vendor code. This is the
  status-quo behavior post-PR-5 of Phase 5.
- **Feature-gated builds** (`cargo build --features dlss,fsr`) link
  the real SDKs and the `supports()` checks return true on the
  appropriate hardware. Used for the local dev experience, the
  shipping editor binary, and the self-hosted GPU runner CI step.

The renderer's `UpscalerRegistry::new` consults the cargo cfg flags
at compile time:

```rust
#[cfg(feature = "dlss")]
registry.register(VendorDlss::real());
#[cfg(not(feature = "dlss"))]
registry.register(VendorDlss::stub());
```

A new public function `UpscalerRegistry::with_phase6_defaults()`
replaces Phase 5's `with_phase5_defaults()` (the old name remains
as a deprecated alias for one release).

### 3. Vendor SDK runtime call shape

Vendor upscalers run in-process (not in a subprocess), but their
*loader* code runs sandboxed:

- **DLSS / Streamline.** `slInit()` is called once at device init
  under a try-catch wrapper (Streamline can fail with platform-
  specific reasons — driver too old, signature mismatch, machine
  in safe mode). Per-frame `slEvaluateFeature` calls run inline.
- **FSR 4.** `ffxFsrContextCreate` runs at device init; per-frame
  `ffxFsrDispatch` runs inline.
- **XeSS 2.** `xessCreateContext` runs at device init; per-frame
  `xessExecute` runs inline.

The loader is what `slInit` / `ffxFsrContextCreate` does internally
— probe shared libraries on disk, validate signatures (Streamline),
allocate GPU memory. The loader code is what historically has had
vendor-side bugs (a malformed driver causing a parser CVE).

The engine's mitigation: **the loader runs in a separate worker
thread** under a `catch_unwind` boundary; a panic or abort during
`slInit` returns the provider to its stub state and logs to the
telemetry channel. The engine continues with the next-priority
provider (FSR after DLSS, XeSS after FSR, owned-ONNX after XeSS).

### 4. Boundary discipline

- **`wgpu::` boundary (ADR-049) remains intact.** The
  `engine-upscale-vendor` crate consumes `engine_gpu::Device`,
  `Texture`, `Buffer` — never raw `wgpu::*`. The vendor SDKs
  themselves expose native GPU interop (Vulkan VkImage handles,
  D3D12 resource handles). `engine-gpu` gains a small interop
  surface (`Texture::as_vk_image()`, `Texture::as_d3d12_resource()`)
  that the vendor wrappers consume; the surface is opaque to other
  crates (it lives behind a `#[cfg(any(feature = "dlss", feature =
  "fsr", feature = "xess"))]` gate).
- **`engine_platform::subprocess` (ADR-019) is used for the SDK
  loader's *signature verification* path only**, not the per-frame
  call. Verification: at engine init, the SDK shared library's
  BLAKE3 digest is checked against a known-good list in
  `crates/engine-upscale-vendor/sdk_digests.toml`. The check runs
  in-process; if it fails, the provider stays stubbed and the
  failure is logged.

### 5. License management

- `deny.toml` gains per-vendor allowances under
  `[licenses.exceptions]`:
  ```toml
  [[licenses.exceptions]]
  name = "streamline-sys"
  allow = ["LicenseRef-NVIDIA-Streamline"]
  ```
  Same for `fsr-sys` (LicenseRef-AMD-FSR-EULA) and `xess-sys`
  (LicenseRef-Intel-XeSS-EULA). The `LICENSE-VENDOR.txt` files in
  each `*-sys` crate are the authoritative copies.
- The license-check CI step (`cargo deny check licenses` in the
  gate job) runs both with and without features in PR 5; both
  must pass.

### 6. Selection cascade unchanged from ADR-005

The runtime cascade (`UpscalerRegistry::select`) remains: DLSS →
FSR → XeSS → OwnedOnnxTemporal → OwnedBilinear. The `SelectionLogger`
records which provider was chosen, why, and at what time. The
`[upscaler]` section of `engine.toml` (added in PR 5; see ADR-005
§Consequences) permits an explicit override:

```toml
[upscaler]
provider = "auto" | "dlss" | "fsr" | "xess" | "owned-onnx" | "owned-bilinear"
quality = "performance" | "balanced" | "quality" | "ultra-quality"
```

## Rationale

- **In-process is mandatory for per-frame perf.** A subprocess would
  add ~16 ms of round-trip IPC per frame — incompatible with 60 FPS.
- **Sandbox the *loader*, not the per-frame call.** The historical
  CVE surface is the SDK's init path (driver / library / signature
  validation). The per-frame `slEvaluateFeature` / `ffxFsrDispatch`
  calls operate on GPU-allocated buffers — much smaller attack
  surface.
- **Cargo features keep CI green without SDKs.** The default no-
  feature build is identical to Phase 5's behavior; the CI matrix
  doesn't have to download vendor SDKs. The self-hosted GPU runner
  builds with `--features all-vendors` and exercises the real paths.
- **Vendored `*-sys` crates avoid drift.** Pulling DLSS from a crates.
  io shim would couple the engine to a third party's release cadence.
  Vendoring per the `tools/upscaler-vendor-sdks/` pattern keeps the
  engine in control of when SDK bumps happen.
- **`engine-gpu` interop API behind a feature gate** preserves the
  ADR-049 boundary for the common case. Other crates cannot reach
  the interop API without explicitly opting into a vendor feature.

## Consequences

- One new crate `engine-upscale-vendor`. Three new `*-sys` crates
  under `tools/upscaler-vendor-sdks/{streamline,fsr,xess}/`. All
  added to workspace `members`.
- `engine.toml` schema gains `[upscaler]` section. The runtime
  reader in `crates/engine-platform/src/manifest.rs` (or wherever
  engine.toml is parsed) gains a new field.
- The `crates/engine-render/src/upscale.rs` stubs are replaced
  with re-exports from `engine-upscale-vendor`. Backward-compat:
  the old type names (`VendorDlss`, etc.) remain at the same
  module path.
- `deny.toml` gains three license exceptions; `LICENSE-VENDOR.txt`
  files in each `*-sys` crate.
- ADR-051 (deviations register) gains an entry per vendor:
  "vendored binary blob with proprietary license — necessary for
  per-frame perf on supported hardware".
- The self-hosted GPU runner step in `.github/workflows/ci.yml`
  installs vendor SDKs and builds with `--features all-vendors`.
  Vendor SDKs are downloaded from a private mirror (not from
  vendor servers per CI invocation) for build reliability.
- `bin/engine-bench-frame-pacing/`'s JSON report gains a
  `"upscaler"` field with the selected provider name + reason.

## Risks and tradeoffs

- **Three external SDK dependencies with independent release
  cadences.** Each SDK has had API breaks (DLSS Streamline 1→2→3,
  FSR 2→3→4). Mitigation: the binding crates are pinned to a
  specific version; bumps require an ADR amendment + PR. Bumps
  are bounded in blast radius (one crate per vendor).
- **Closed-source loader code in the editor process.** Mitigated by
  the loader-thread sandbox + signature digest check. A truly
  malicious loader could still attack the editor; the BLAKE3
  digest tracking provides post-hoc detection.
- **Cargo feature gates add CI complexity.** The matrix grows: each
  feature combination is a separate build job. Mitigation: PR 5's
  CI changes test the four canonical configurations (no features,
  --features dlss, --features fsr, --features all-vendors); other
  combinations are inferred-safe.
- **Vendor SDK licenses may change.** A license change that disallows
  redistribution would force a re-architecture. Mitigation: ADR-051's
  per-vendor entry includes a revisit gate ("if NVIDIA reposts
  Streamline under a non-redistributable license").
- **Per-machine driver-version dependency.** A user with an outdated
  driver gets `supports() == false` for DLSS even with hardware
  support. Mitigated by the cascade (FSR / XeSS / owned-ONNX as
  fallback) + clear telemetry log of *why* DLSS was skipped.

## Alternatives considered

- **Owned upscalers only (no vendor SDKs).** Forfeits the
  hardware-accelerated quality on supported GPUs. Rejected by
  ADR-005.
- **One unified vendor SDK shim (Streamline acts as a unifier).**
  Streamline does include some cross-vendor abstractions but its
  FSR / XeSS coverage lags vendor-native; using each vendor's
  own SDK is what their docs recommend. Rejected.
- **WASM-sandboxed loaders.** Same problem as ADR-019's WASM-
  importer alternative: native interop is the point, WASM forfeits
  it. Rejected.
- **Subprocess per upscaler.** The per-frame IPC cost (>10 ms) is
  the rejection — incompatible with the frame-pacing milestone.
  Rejected.
- **One mega-crate `engine-upscale-vendor-all`** instead of three
  `*-sys` crates. Larger blast radius on a single CVE; harder to
  selectively bump versions. Rejected.

## Verification

- Implementation lands in Phase 6 PR 5. Test files:
  - `crates/engine-upscale-vendor/tests/dlss_supports_stub.rs`:
    without `--features dlss`, `VendorDlss::supports()` returns
    false on any device.
  - `crates/engine-upscale-vendor/tests/dlss_supports_real.rs`:
    with `--features dlss`, mocks the Streamline FFI return values
    and asserts the `supports()` function correctly classifies
    each tier of NVIDIA hardware. (Real hardware test runs only on
    the self-hosted runner.)
  - Equivalent tests for FSR, XeSS.
  - `crates/engine-upscale-vendor/tests/cascade.rs`: registry built
    with stub providers; assert selection cascade order; assert
    `SelectionLogger` records each skipped provider with reason.
  - `crates/engine-upscale-vendor/tests/sdk_digest_check.rs`: feed
    a stub SDK library with a known digest; assert pass. Feed an
    altered library; assert fail + provider stubbed.
- CI:
  - The gate job tests with `--no-default-features`.
  - The self-hosted GPU runner step (PR 6) runs with `--features
    all-vendors` and exercises the real cascade on the original CI runner
    (FSR path active, DLSS/XeSS skipped due to GPU brand).
- Telemetry (ADR-010): `SPAN "render.upscale"`,
  `COUNTER "render.upscale.skipped"` per provider,
  `GAUGE "render.upscale.selected_provider"` (string-tagged).
- The frame-pacing JSON report includes the selected upscaler in
  every measurement run.
- ADR-051 gets a sibling-PR amendment in the same PR 5 commit
  adding the three vendor entries.
