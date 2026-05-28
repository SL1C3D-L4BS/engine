# NVIDIA Streamline 2.x SDK · vendor sandbox

Per ADR-079 §1. The SDK is vendored under this directory after the
user / CI runner follows the fetch + verify procedure in
`docs/runbooks/vendor-upscaler-sdks.md`.

## Layout (post-fetch)

```
streamline/
├── LICENSE-VENDOR.txt   # NVIDIA Streamline EULA
├── BLAKE3.txt           # per-binary digest manifest
├── README.md            # this file
├── sl/                  # NVIDIA Streamline SDK tree (headers + binaries)
└── streamline-sys/      # bindgen-generated Rust FFI wrapper
    ├── Cargo.toml
    ├── build.rs
    └── src/lib.rs
```

## Activation

The `engine-upscale-vendor` crate declares `streamline-sys` as an
optional dependency behind the `dlss` cargo feature. The default
workspace build does not link NVIDIA code:

```sh
cargo build --workspace                           # no NVIDIA code
cargo build --workspace --features dlss           # links Streamline
cargo build --workspace --features all-vendors    # links DLSS + FSR + XeSS + ORT
```

## License

NVIDIA Streamline SDK License Agreement (`LICENSE-VENDOR.txt`). See
ADR-051 entry 5 for the acknowledged-deviation engineering record.
