# Vendor upscaler SDKs — fetch, verify, build (ADR-079 §7)

This runbook covers the procedure for vendoring the three upscaler
SDKs the Phase 6 cascade uses: NVIDIA Streamline 2.x (DLSS), AMD
FidelityFX FSR 4, and Intel XeSS 2.

The baseline workspace build (no cargo features) links no vendor
code. Activation is opt-in per ADR-079 §5:

```sh
cargo build --workspace                            # baseline, no SDKs linked
cargo build --workspace --features dlss            # links NVIDIA Streamline
cargo build --workspace --features fsr             # links AMD FSR 4
cargo build --workspace --features xess            # links Intel XeSS 2
cargo build --workspace --features ort-runtime     # links ONNX Runtime
cargo build --workspace --features all-vendors     # links all four
```

## 1 · NVIDIA Streamline 2.x (DLSS / Reflex / DLAA)

1. Visit https://developer.nvidia.com/rtx/streamline and request
   the latest Streamline 2.x source release (registration required;
   NVIDIA EULA applies).
2. Extract the tarball under
   `tools/upscaler-vendor-sdks/streamline/sl/`. The top-level
   directory should contain `include/`, `lib/`, `samples/` etc.
3. Compute BLAKE3 digests for every binary file inside `sl/`:

   ```sh
   cd tools/upscaler-vendor-sdks/streamline/sl
   find . -type f -not -path '*/\.*' -print0 \
     | xargs -0 -n 1 -I {} sh -c 'echo "$(b3sum {} | awk "{print \$1}") {}"' \
     > ../BLAKE3.txt
   ```

4. Verify `LICENSE-VENDOR.txt` matches NVIDIA's EULA verbatim.
5. Re-run bindgen (`cd streamline-sys && cargo build --release`).
   `build.rs` performs the digest verify automatically; a mismatch
   fails the build with a re-fetch directive.

## 2 · AMD FidelityFX FSR 4 SDK

1. Clone the public FFX SDK release tag (MIT licensed):

   ```sh
   cd tools/upscaler-vendor-sdks/fsr
   git clone --branch v4.0.0 \
     https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK ffx-sdk
   ```

2. Generate the digest manifest:

   ```sh
   cd ffx-sdk && find . -type f -not -path '*/\.git/*' -print0 \
     | xargs -0 -n 1 -I {} sh -c 'echo "$(b3sum {} | awk "{print \$1}") {}"' \
     > ../BLAKE3.txt
   ```

3. `fsr4-sys`'s `build.rs` rebuilds bindings.

## 3 · Intel XeSS 2 SDK

1. Clone the public XeSS release tag (MIT licensed):

   ```sh
   cd tools/upscaler-vendor-sdks/xess
   git clone --branch v2.0.0 https://github.com/intel/xess xess-sdk
   ```

2. Generate the digest manifest (same `find … b3sum` pipeline as
   above) into `BLAKE3.txt`.
3. `xess2-sys` build picks up the SDK.

## 4 · ONNX Runtime

The ORT runtime is bound via the `ort` crate (workspace dep) — no
vendor sandbox required, the crate handles platform binaries. The
trained model artifact lives at
`crates/engine-render/assets/onnx/temporal_upscaler_v1.onnx` (Git
LFS tracked). See `tools/onnx-train/README.md` for the retraining
workflow.

## 5 · Verification

After fetching all four runtimes:

```sh
cargo build --workspace --features all-vendors
cargo test --workspace --features all-vendors -- --test-threads 1
```

The `engine-bench-frame-pacing` binary reports the active cascade
via its `build_info` JSON field:

```json
{
  "build_info": {
    "dlss": true,
    "fsr": true,
    "xess": true,
    "ort_runtime": true
  }
}
```

On the user's RX 580 (Polaris GFX8) the runtime cascade still
lands on `vendor.fsr` (EASU spatial) because the device declines
the FSR 4 tensor probe; the runtime cascade selection is hardware-
driven, not feature-flag-driven.

## 6 · CI runner provisioning

The full-vendor build is **not** part of the required CI gate. The
self-hosted runner can opt in by setting
`CARGO_BUILD_FEATURES=all-vendors` in its job spec, but the
baseline GitHub-hosted runners stay feature-less so they never
need vendor SDKs. ADR-079 §5 documents this discipline.

## 7 · License hygiene

`deny.toml` carries three license-fingerprint allowances after PR 3
lands:

- `LicenseRef-NVIDIA-Streamline` (the NVIDIA EULA fingerprint).
- The MIT fingerprint (already allowed for FSR + XeSS).

A new vendor SDK requires both a `LICENSE-VENDOR.txt` entry and a
`deny.toml` allowance.

## References

- ADR-079 — Vendor SDK FFI discipline (parent).
- ADR-066 — Cascade order.
- ADR-076 — FSR-EASU spatial fallback (in-tree, always available).
- ADR-051 — Acknowledged deviations register (entries 5, 6, 7).
- ADR-067 + ADR-080 — ONNX runtime + training pipeline.
