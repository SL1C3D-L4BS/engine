# [ENGINE] Platform

A Rust-first, zero-runtime-dependency game engine and platform.

This repository is the monorepo skeleton scaffolded in **Phase 0**. The crate
tree, levels, and architecture follow the authoritative specification:

> `~/Resources/documentation/ENGINE_SPECIFICATION_v2.0.md`

## Layout

- `crates/` — the 19 `engine-*` library crates (levels 0–4, spec Part IV.1)
- `testbed/engine-raster/` — software rasterizer, the rendering oracle (Part IX)
- `tools/`, `bin/` — CLI and TUI tool crates (Parts VII, VIII)
- `tests/{integration,rendering,semver,bench,determinism}/` — test surfaces
- `docs/adr/` — Architecture Decision Records (Part XXII)
- `docs/architecture/` — subsystem architecture docs

## Build

```sh
just build      # compile the workspace
just ci         # full pre-push gate (build, test, lint, fmt, deny)
```

The toolchain is pinned in `rust-toolchain.toml`; `sccache` and the `mold`
linker are configured in `.cargo/config.toml`.

## Status

Phase 0 — environment and skeleton. The engine itself is built across
Phases 1–11; see spec Part XXI.
