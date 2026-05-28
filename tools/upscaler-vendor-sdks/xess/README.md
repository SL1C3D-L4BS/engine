# Intel XeSS 2 SDK · vendor sandbox

Per ADR-079 §1. MIT licensed. The SDK is vendored after the fetch
+ verify procedure in `docs/runbooks/vendor-upscaler-sdks.md`.

XeSS 2 has two sub-paths:

- **XMX** — XMX matrix accelerator on Arc B+ (Battlemage) and newer.
- **DP4a** — cross-vendor INT8 path; runs on most NVIDIA Turing+ and
  AMD RDNA+ GPUs.

The `xess` feature on `engine-upscale-vendor` activates the runtime;
`VendorXess::supports()` reports the live path.

## Activation

```sh
cargo build --workspace --features xess
```
