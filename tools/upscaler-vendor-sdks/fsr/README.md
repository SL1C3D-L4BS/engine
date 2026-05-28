# AMD FidelityFX FSR 4 SDK · vendor sandbox

Per ADR-079 §1. MIT licensed end-to-end. The SDK is vendored after
the fetch + verify procedure in
`docs/runbooks/vendor-upscaler-sdks.md`.

The FSR 4 tensor path applies only on RDNA 4 hardware; on every
other host the cascade selects ADR-076's in-tree FSR-EASU spatial
upsampler (which ships unconditionally as the default path inside
`engine-render`).

## Activation

```sh
cargo build --workspace --features fsr            # links FSR 4 SDK
```
