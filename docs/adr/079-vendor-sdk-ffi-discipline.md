# ADR-079 — Vendor SDK FFI discipline (DLSS Streamline 2.x + FSR 4 + XeSS 2)

- Status: Accepted (Phase 6 design contract; implementation lands in
  Phase 6 PR 3)
- Date: 2026-05-28
- Phase: 6 — NEURAL RENDERING & GAUSSIAN SPLATTING
- Companion: ADR-005 (vendor upscaler trait), ADR-018 (plugin
  sandboxing — the same loader discipline), ADR-019 (asset sandbox
  subprocesses), ADR-051 (acknowledged deviations register — three
  new entries land via this ADR), ADR-066 (upscaler vendor cascade),
  ADR-076 (FSR EASU spatial fallback), ADR-084 (Phase 6 PR slicing)

## Context

ADR-066 locked the cascade order — `DLSS → FSR → XeSS →
OwnedOnnxTemporal → OwnedBilinear`. Phase 5 PR 5 shipped stub
providers (`VendorDlss`, `VendorFsr`, `VendorXess`) whose
`supports()` returned `false`. ADR-076 amended `VendorFsr::supports()`
to `true` via the in-tree EASU spatial fallback (no SDK; pure WGSL
implementation of the published FSR 1.0 algorithm). The other two
vendors (DLSS, XeSS) and the *tensor-accelerated* FSR 4 path remain
behind `supports() == false`.

Phase 6 closes that gap. Three vendor SDKs are integrated:

- **NVIDIA Streamline 2.x** (the DLSS / Reflex / DLAA wrapper).
  Source-distributable under NVIDIA's permissive terms; binary
  blobs (the actual `nvngx_dlss.dll` / `libnvngx_dlss.so`) are not
  vendored and ship via the user's NVIDIA driver install.
- **AMD FSR 4 SDK** (FidelityFX SDK with the RDNA-4 tensor path).
  MIT licensed; vendorable end-to-end.
- **Intel XeSS 2 SDK**. MIT licensed; vendorable end-to-end. The
  SDK ships both the XMX-accelerated path (Arc B+) and the DP4a
  cross-vendor path.

The integration discipline must:

1. Keep the `wgpu::` boundary intact (ADR-049). The `*-sys` crates
   sit alongside `engine-render`'s ADR-049-blessed surface; they
   never re-export wgpu types.
2. Compile out by default. `--features dlss / fsr / xess` are
   opt-in; the no-features build has no vendor SDK linkage. This
   keeps the workspace's "no opaque binary deps unless explicitly
   activated" property (ADR-025).
3. Run loader threads in a sandbox. SDK init runs inside a
   `engine_platform::sandbox::Sandbox` thread so a crashing SDK
   does not bring down the renderer (mirrors ADR-018's plugin
   sandbox + ADR-019's asset subprocess pattern).
4. Verify SDK digests. Each vendored SDK has a `LICENSE-VENDOR.txt`
   + a `BLAKE3.txt` digest manifest. The build script verifies the
   digest matches before linking.

## Decision

### 1. Three `*-sys` crates

```
tools/upscaler-vendor-sdks/
├── streamline/
│   ├── LICENSE-VENDOR.txt        # NVIDIA Streamline SDK License
│   ├── BLAKE3.txt                # vendor-side digest of each binary
│   ├── README.md                 # SDK fetch + verify procedure
│   ├── sl/                       # Streamline SDK source/binaries
│   └── streamline-sys/           # Rust bindgen wrapper
│       ├── Cargo.toml
│       ├── build.rs              # bindgen + link directives
│       └── src/lib.rs            # auto-generated FFI bindings
├── fsr/
│   ├── LICENSE-VENDOR.txt        # AMD FidelityFX MIT
│   ├── BLAKE3.txt
│   ├── README.md
│   ├── ffx-sdk/                  # FidelityFX SDK source (vendored)
│   └── fsr4-sys/
│       ├── Cargo.toml
│       ├── build.rs
│       └── src/lib.rs
└── xess/
    ├── LICENSE-VENDOR.txt        # Intel XeSS 2 SDK MIT
    ├── BLAKE3.txt
    ├── README.md
    ├── xess-sdk/                 # XeSS 2 SDK source (vendored)
    └── xess2-sys/
        ├── Cargo.toml
        ├── build.rs
        └── src/lib.rs
```

Each `*-sys` crate is a workspace member only when its feature is
on; the workspace `Cargo.toml` includes them inside
`[workspace.metadata.cargo-features]` conditionals (or, per
cargo-2026 surface, listed via `[workspace.members]` + opt-in via
the consuming crate's feature gating — the actual mechanism is
chosen at PR 3 implementation time per cargo's current best
practice for conditional members).

### 2. License management

Per ADR-066 §License management:

- `LICENSE-VENDOR.txt` is the verbatim vendor EULA / MIT, committed
  to the repo.
- `BLAKE3.txt` is a manifest listing each binary in the vendor SDK
  (e.g. `streamline_sdk.lib`, `libfidelityfx_fsr4.so`,
  `libxess2_dp4a.so`) and its expected BLAKE3 digest.
- `build.rs` verifies the digest before invoking bindgen. A digest
  mismatch fails the build with a clear "vendored SDK has been
  modified; refetch from <vendor URL> + re-verify" message.
- `deny.toml` grows three license-exception entries — one per
  vendor — naming the license fingerprint the workspace accepts.

### 3. Bindgen discipline

The `*-sys` crates use `bindgen 0.69+` to generate FFI bindings
from the SDK's C headers. Bindgen invocations are reproducible:

- Pinned `bindgen` version (workspace dep).
- `--no-derive-debug --no-derive-default --no-derive-copy` to keep
  the generated surface minimal.
- Output is committed to `src/lib.rs` (not regenerated per build) so
  the build cost is constant and the FFI surface is reviewable. The
  `build.rs` script *verifies* the bindings match the SDK header
  hash; a regenerate-and-commit ritual is part of the runbook.

### 4. Loader-thread sandbox

`engine-upscale-vendor` provides a shared loader helper:

```rust
// crates/engine-upscale-vendor/src/loader.rs

pub struct VendorLoader<F: FnOnce() -> Result<T, E> + Send + 'static, T, E> {
    thread: JoinHandle<Result<T, E>>,
}

impl VendorLoader { /* spawn into engine_platform::sandbox; reap with timeout */ }
```

Every vendor SDK's `slInit` / `ffxFsrContextCreate` / `xessCreateContext`
runs inside the loader thread with a 5-second timeout. If the SDK
hangs, the loader times out and the provider's `supports()` returns
`false` with a `SelectionLogger::log_init_timeout(...)` callback.

### 5. Per-vendor cargo features (unchanged from ADR-066 §6)

```toml
# crates/engine-upscale-vendor/Cargo.toml
[features]
default = []
dlss      = ["dep:streamline-sys"]
fsr       = ["dep:fsr4-sys"]
xess      = ["dep:xess2-sys"]
ort-runtime = ["dep:ort"]
all-vendors = ["dlss", "fsr", "xess", "ort-runtime"]
```

The default build links no vendor SDKs. CI's baseline build is
`--no-default-features --workspace`. A separate optional CI job (out
of scope for this ADR's enforcement) builds `--features all-vendors`
on a runner with the SDKs present; this is documented in the
runbook but not added to the required CI gate.

### 6. ADR-051 amendments — three new entries

This ADR adds three entries to the deviations register, landing in
PR 3 of Phase 6:

- **Entry 5** — DLSS Streamline SDK (NVIDIA EULA-gated; user opt-in
  via `dlss` feature).
- **Entry 6** — AMD FSR 4 SDK (MIT; FidelityFX vendored end-to-end).
- **Entry 7** — Intel XeSS 2 SDK (MIT; XeSS vendored end-to-end).

Each entry follows ADR-051 §5's amendment format. Entry 5 is the
sensitive one: NVIDIA's Streamline source distribution is permissive
but the runtime DLL ships with the user's driver, not with the
engine — the deviation entry captures this load-time discipline.

### 7. SDK fetch procedure (runbook)

A new `docs/runbooks/vendor-upscaler-sdks.md` runbook documents:

1. Where to fetch each SDK (NVIDIA developer portal; GPUOpen
   FidelityFX SDK release page; Intel Developer Zone XeSS page).
2. How to verify the fetched binaries' digests against
   `BLAKE3.txt`.
3. How to re-run bindgen if the SDK is updated (the digest manifest
   is the trigger).
4. The cargo-feature activation pattern for development + CI
   runners.

The runbook is read-only; the SDK *contents* themselves are
vendored under `tools/upscaler-vendor-sdks/<vendor>/` per the
directory layout above.

### 8. Real provider implementations

`crates/engine-upscale-vendor/src/{dlss,fsr,xess}.rs` replace the
Phase-5 stubs:

```rust
// crates/engine-upscale-vendor/src/dlss.rs

impl UpscalerProvider for VendorDlss {
    fn supports(&self, _: &engine_gpu::Device) -> bool {
        #[cfg(feature = "dlss")]
        { /* call streamline_sys::slIsSupported(...) inside VendorLoader */ }
        #[cfg(not(feature = "dlss"))]
        { false }
    }
    fn upscale(&self, ctx: &mut UpscaleCtx<'_>) -> Result<UpscaleResult, UpscaleError> {
        #[cfg(feature = "dlss")]
        { /* build frame token, invoke DLSS-SR, copy result */ }
        #[cfg(not(feature = "dlss"))]
        { Err(UpscaleError::ProviderUnavailable) }
    }
}
```

The cargo-feature gating compiles the SDK call sites out entirely
when the feature is off; the no-feature build has zero references
to vendor symbols. `cargo build --no-default-features --workspace`
must remain green.

## Consequences

### Positive

- The cascade (`DLSS → FSR → XeSS → OwnedOnnxTemporal → OwnedBilinear`)
  has real implementations at all five levels. On the user's RX
  580, the cascade lands at `vendor.fsr` (EASU spatial per ADR-076).
  On a hypothetical RTX 40+ host, it lands at `vendor.dlss`.
- The per-vendor cargo-feature gating means default-build users
  pay zero compile-time + zero binary-size cost for SDKs they
  don't use.
- The loader-thread sandbox means a crashing SDK does not crash
  the renderer.
- The runbook + digest manifest mean SDK updates are reviewable
  + reproducible.

### Negative

- Three new `*-sys` crates increase the workspace's surface. Each
  is a thin bindgen wrapper; the *generated* code lives in tree
  for review; total LOC is moderate (~5k for all three combined,
  most of which is bindgen output).
- NVIDIA's Streamline SDK has a non-MIT license. The
  `LICENSE-VENDOR.txt` + ADR-051 §5 amendment document this
  discipline; users opt in via the `dlss` cargo feature, and the
  baseline CI build remains license-clean.
- Loader-thread sandboxing adds 5-second startup latency for the
  vendor init *if* the SDK has to be reloaded — mitigated by
  caching the supports() result for the process lifetime.

### Neutral

- The runbook is the source of truth for "how to fetch + verify
  vendor SDKs". The ADR is the engineering contract; the runbook
  is the operational guide.

## Implementation

PR 3 of Phase 6 (per ADR-084):

1. `tools/upscaler-vendor-sdks/{streamline,fsr,xess}/` — vendored
   SDK trees + `*-sys` crates.
2. `crates/engine-upscale-vendor/src/{dlss,fsr,xess}.rs` — real
   implementations replacing stubs.
3. `crates/engine-upscale-vendor/src/loader.rs` — shared
   loader-thread helper.
4. `docs/runbooks/vendor-upscaler-sdks.md` — fetch + verify
   procedure.
5. `docs/adr/051-acknowledged-deviations.md` — entries 5, 6, 7.
6. `deny.toml` — per-vendor license fingerprint allowances.

## References

### SDKs

- NVIDIA Streamline 2.x — <https://developer.nvidia.com/rtx/streamline>.
- AMD FidelityFX SDK (FSR 4) — <https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK>.
- Intel XeSS 2 SDK — <https://github.com/intel/xess>.

### Books

- *Real-Time Rendering 4* — Ch. 23 (Graphics Hardware) for the
  per-vendor tensor-accelerator surface.
- *Game Engine Architecture* (Gregory) — Ch. 6.7 (Plug-in
  Architectures) for the loader-thread sandbox pattern.

### Prior engine ADRs

- [ADR-005](005-upscaler-provider-trait.md) — the trait surface
  this ADR populates with real implementations.
- [ADR-018](018-plugin-sandboxing.md) — the sandbox pattern the
  loader thread mirrors.
- [ADR-019](019-asset-sandbox-subprocesses.md) — companion sandbox
  precedent.
- [ADR-049](049-engine-gpu-wgpu-wrapper.md) — the wgpu-boundary
  rule this ADR's `*-sys` crates respect.
- [ADR-051](051-acknowledged-deviations.md) — gains three new
  entries via this ADR.
- [ADR-066](066-upscaler-vendor-cascade.md) — the cascade order
  this ADR's real implementations populate.
- [ADR-076](076-fsr-2-spatial-fallback.md) — the EASU fallback
  that runs when the FSR 4 tensor path is unavailable.
- [ADR-084](084-phase-6-pr-slicing.md) — Phase 6 PR slicing.
