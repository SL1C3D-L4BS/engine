# ADR-003 — Slang as the authoring shader language

- Status: Accepted
- Date: 2026-05-18 (expanded 2026-05-24 per audit §15 Phase-0 ADR sweep)
- Phase: 0 (pre-v1.0 — contract-exempt per risk R-03)
- Companion: ADR-037 (slangc toolchain — concrete realisation),
  ADR-038 (Slang reproducibility), ADR-006 (WGSL + WebTransport for
  the web target), ADR-019 (asset sandbox subprocesses)

## Context

The engine targets multiple graphics backends in production: Vulkan
on Linux/Windows native (spec finding F-07), WebGPU on the web
target (ADR-006), Metal on macOS, DirectX 12 on Windows when the
platform calls for it. Authoring shaders once per platform — five
near-identical HLSL/WGSL/MSL/GLSL/SPIR-V variants of each shader —
is the historical cost of multi-backend engines and the source of
their slow shader-iteration loops.

Three options for the authoring language:

- **HLSL** — DXC ships SPIR-V/DXIL emission; the WGSL/MSL path is
  weak (Naga only partially translates HLSL→WGSL; MSL requires a
  separate Microsoft tool). Maintained by Microsoft; Khronos has
  no governance role.
- **WGSL** — the web target's lingua franca; Naga compiles it to
  SPIR-V/HLSL/MSL. Weak generics; no modules; the spec
  ecosystem is young.
- **Slang** — Khronos-hosted (since 2024), single source compiling
  to SPIR-V / WGSL / HLSL / MSL / GLSL via `slangc`. First-class
  generics, modules, automatic differentiation, and a reflection
  layer the engine can consume.

## Decision

The engine authors all shaders in Slang. `slangc` is invoked as a
sandboxed subprocess (ADR-019 / ADR-037) at asset-build time. The
shader pak (ADR-008's content-addressed format) stores the
SPIR-V/WGSL/MSL/HLSL outputs alongside the Slang source hash.

WGSL is retained as a first-class *output* target — the web build
loads pre-compiled WGSL out of the shader pak, no `slangc` at
runtime. The pak's content addressing guarantees that the WGSL on
disk is exactly the WGSL Slang produced from the committed source.

## Rationale

Khronos governance (vs. Microsoft governance of HLSL) is the
50-year longevity argument applied to the shader layer (ADR-012
on the API stability contract): the engine cannot afford a
shader-language vendor that may deprioritise the language on a
five-year horizon.

Slang's generics are not cosmetic — the engine's PBR material
system (Phase 5 PR 3) is expressed as a single Slang module
generic over `BindlessTextureId` indices, instantiated by the
asset pipeline for every material variant. HLSL's macro
metaprogramming would need ~3× the LOC for equivalent variant
coverage; WGSL would need a separate per-variant authored file.

The autodiff feature is forward-looking — Phase 11+ might use it
for differentiable rendering research (the spec's NeRF/3DGS
mentions in §IV.4.B). Slang ships it today; HLSL/WGSL do not.

## Consequences

- `tools/engine-shader/` houses the slangc subprocess wrapper
  (ADR-037 / ADR-038). The toolchain is a vendored, version-
  pinned `slangc` binary in `vendor/` per the spec's owned-
  toolchain stance.
- The shader pak's content-addressed format (ADR-008) includes
  the Slang source SHA-256 + slangc version + target backend, so
  any change to source or compiler invalidates the cached output
  deterministically.
- A Slang-only authoring layer means contributors learn Slang
  rather than HLSL/GLSL/WGSL/MSL. The reference library
  (`docs/architecture/engine-shader.md` — to be authored alongside
  ADR-037 once a stable contributor onboarding writeup exists)
  is the entry point.
- The web build does not need a WASM-targeted slangc — WGSL is
  pre-compiled into the pak.

## Risks and tradeoffs

- **`slangc` is a vendored binary.** Mitigation: version-pinned,
  checksum-verified at build time, the reproducibility golden
  (ADR-038) catches any silent compiler swap.
- **Slang ecosystem is younger than HLSL/GLSL.** Tooling
  (editor LSP, formatter, linter) is less mature. Mitigation:
  Khronos is investing; the engine's needs are narrow enough
  (the shader-graph editor is Phase 10) that the current state
  is sufficient through Phase 5.
- **Subprocess invocation cost.** Per-shader compile is ~50–200 ms;
  mitigated by the asset pipeline's content-addressed caching
  (re-compile only on source or compiler change) and by parallel
  invocation through `engine-platform::JobGraph` (ADR-032).
- **Reflection drift.** Slang's reflection JSON format could
  change between versions. Mitigation: ADR-038's reproducibility
  golden pins the JSON schema; a schema change is a deliberate
  PR, not a silent update.

## Alternatives considered

- **HLSL + DXC.** Real and mature; weak WGSL/MSL story; vendor
  governance instead of consortium governance. Rejected.
- **WGSL as the source-of-truth, with Naga emitting other
  backends.** Naga's WGSL→SPIR-V/HLSL/MSL path works; the
  WGSL→generic-shader story (no generics, no modules) doesn't
  scale to the engine's material system. Rejected.
- **Multi-source maintenance** (an HLSL file + a WGSL file + an
  MSL file per shader). Standard pre-Slang practice; the
  authoring cost is unacceptable for the engine's surface.
  Rejected.
- **Owned shader language.** The R-02 "own the layer" stance
  considered but rejected per the same reasoning as ADR-025
  (audited crypto): Slang is a well-governed standard with a
  Khronos-led ecosystem; owning a shader language is not the
  best use of foundation effort.

## Verification

- `cargo test -p engine-shader` — slangc subprocess produces
  expected outputs for the corpus of test shaders.
- `tools/engine-shader` reproducibility golden (ADR-038) — bytewise
  identical SPIR-V/WGSL/MSL/HLSL for a given Slang source +
  slangc version. The golden is part of CI's determinism gate.
- The shader pak format's `slang_source_sha256` + `slangc_version`
  fields are inspected by `engine-tui` (Phase 10) to surface
  shader provenance to developers.
- WGSL output is loadable by the web target's WebGPU runtime
  (verified by Phase 9 web-target work; pre-Phase-9, by the
  Naga validator in CI).
