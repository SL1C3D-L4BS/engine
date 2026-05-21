# ADR-037 · Slang shader toolchain: subprocess shape and asset bundle

- Status: Accepted (PR 4 of Phase 4)
- Date: 2026-05-20
- Phase: 4 (SCRIPTING — spec Part XXI)

## Context

Phase 4 PR 4 lands the shader toolchain that unblocks the Phase-5
renderer. The engine spec ADR-003 names Slang as the source language
for shaders. Owning that language is **out of scope** — re-implementing
HLSL plus Slang's generics machinery would be a multi-year side-quest
and would diverge from the canonical compiler the Khronos community
already uses.

The toolchain therefore wraps the official `slangc` binary as a
sandboxed subprocess. The engine owns:

- The artefact format (`Bundle` + `Artifact` codec).
- The asset-pipeline integration (`impl Asset for Bundle`).
- The compile-frame: targets, stages, entry routing, error mapping.
- The reproducibility contract (ADR-038).

Four canonical targets ride through the pipeline: SPIR-V (Vulkan),
WGSL (WebGPU / wgpu), DXIL (DirectX 12), MSL (Metal). Each is a
distinct `slangc -target` invocation.

## Decision

### Subprocess shape (`tools/engine-shader/src/slangc.rs`)

`slangc` is invoked under ADR-019's sandboxed-subprocess rules:

- **Absolute path resolution.** `Compiler::locate` walks `$PATH`
  once, or accepts an explicit `SLANGC` env override. Subsequent
  invocations use the resolved absolute path — no shell, no
  late `$PATH` lookup.
- **Cleared environment.** `Command::env_clear()` on every spawn,
  with `LANG=C.UTF-8` forwarded explicitly. A hostile or unusual
  user env cannot perturb output.
- **Closed stdin.** `Stdio::null()` so a hung pipe cannot stall
  the build. Stdout/stderr piped and captured.
- **Explicit args.** `-target`, `-stage`, `-entry`, `-o <out>`,
  `-reflection-json <refl>`, source path. No flag is constructed
  from user-supplied strings — the toolchain decides every flag.
- **Temp-file IO.** SPIR-V (and DXIL) refuse to write to a pipe;
  the wrapper uses a per-pid + monotonic-counter file in
  `std::env::temp_dir()`, reads the bytes back, and unlinks.

Per-invocation error types (`SlangcError`) discriminate
"slangc not installed" (graceful skip in oracles) from "slangc
ran and rejected the source" (mapped to a `Compile { stage,
target, exit_code, stderr }` payload that surfaces the captured
stderr verbatim).

### Artefact format (`tools/engine-shader/src/artifact.rs`)

```text
Bundle:
  magic      "SHDR"            // 4 B
  version    u16 LE = 1
  stage_tag  u8                // Stage::tag
  entry_len  u16 LE
  entry      [u8; entry_len]
  count      u8                // 1..=4 artifacts
    artifact_0 ... artifact_n

Artifact:
  target_tag u8                // Target::tag
  bytes_len  u32 LE
  bytes      [u8; bytes_len]
  refl_len   u32 LE
  reflection [u8; refl_len]    // slangc reflection JSON
  digest     [u8; 32]          // BLAKE3 over `bytes`
```

Tags are stable across versions: SPIR-V=1, WGSL=2, DXIL=3, MSL=4 for
targets; Vertex=1, Fragment=2, Compute=3 for stages. Re-numbering is
a breaking change to the asset pak.

The on-disk digest is re-hashed and compared on decode; a mismatch
is `DecodeError::DigestMismatch` (the asset pipeline's content-
addressed invariant survives even local corruption).

### Asset-pipeline integration

`impl engine_asset::Asset for Bundle` decodes the on-disk record
via `artifact::decode`. The renderer (Phase 5) consumes
`Handle<Bundle>` and dispatches to the per-target artefact at
device-init time.

### CLI shape (`tools/engine-shader/src/main.rs`)

Owned arg parser, no `clap`. The CLI lives behind the same crate
as the library — one binary, one library, no duplicated logic.
Two modes: single-target (`-t <target>`) and all-targets
(default; failures per target collected, build continues unless
every target failed).

## Consequences

### Positive

- The engine owns the artefact format, the asset-pipeline bridge,
  and the compile-frame. The single thing the engine outsources is
  the surface-language semantics, and that's the place where
  `slangc` is the right tool.
- The subprocess shape is auditable in one file (`slangc.rs`).
- Per-target failures (DXIL on Linux because of missing
  `dxcompiler`) are isolated: SPIR-V, WGSL, and MSL still ship.
- The artefact format is owned, little-endian, and content-addressed
  (BLAKE3 over bytes). It composes with the existing pak pipeline
  without further changes.

### Negative

- A pinned external binary is now a build dependency for engineers
  who want to author shaders. The repo's existing pre-flight script
  records `slangc` in the dependency check.
- DXIL targets need Microsoft's `dxcompiler` shared library, which
  ships only with the Windows DX SDK. The CI matrix records the DXIL
  skip on Linux runners; a Windows runner picks up the digest under
  ADR-038's golden.

### Caveats

- The owned mini-Slang front-end the engine could in theory ship is
  out of scope here and forever — spec ADR-003 names slangc as the
  authoritative source-language compiler. Owning that surface
  diverges from the Khronos community's evolution of the language.

## See also

- ADR-019 — sandboxed subprocess pattern.
- ADR-038 — Slang reproducibility golden.

## Owner

Sliced Engine team. PR 4 in the Phase-4 sequence; closes Phase 4.
