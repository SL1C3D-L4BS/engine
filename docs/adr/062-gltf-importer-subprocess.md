# ADR-062 — glTF importer as sandboxed subprocess

- Status: Accepted (Phase 6 design contract; implementation lands in
  Phase 6 PR 1)
- Date: 2026-05-27
- Phase: 6 — RENDERING FOUNDATION (Track A, Part 2)
- Companion: ADR-019 (asset sandbox subprocesses — pattern parent),
  ADR-037 (slangc subprocess — concrete precedent), ADR-045 (texture
  compress — Phase 5 subprocess precedent), ADR-061 (EMSH + EMAT
  output format), ADR-068 (Phase 6 PR slicing)

## Context

ADR-061 fixes the runtime asset format (EMSH + EMAT). Content
authoring, however, lives outside the engine — artists export from
Blender / Maya / 3ds Max as glTF 2.0 (the de facto interchange
format in 2026). The engine therefore needs an *importer* that
converts glTF → EMSH + EMAT artefacts ingestible by the asset pak.

ADR-019 makes this an asset sandbox subprocess decision. Its threat
model directly applies: a glTF file is untrusted input; its embedded
buffers (`bin`), embedded textures (PNG/JPEG/KTX2), and JSON metadata
are all parse surfaces with historical CVE exposure. A direct
in-editor `gltf` crate invocation is one zero-day away from arbitrary
code execution inside the editor process.

The engine has two concrete prior implementations of the ADR-019
pattern:

1. `tools/engine-shader/` wraps `slangc` (ADR-037). One file
   (`tools/engine-shader/src/slangc.rs`) handles spawn, env-clear,
   stdin-null, temp-file IO, error mapping.
2. `tools/engine-tex-compress/` wraps `intel_tex_2` (ADR-045) into a
   subprocess CLI that emits ETEX. Same shape; the third-party
   library is statically linked into the subprocess binary and never
   loaded into the editor.

`tools/engine-mesh-import/` follows that pattern.

## Decision

### 1. New tool: `tools/engine-mesh-import/`

A workspace member; single-binary crate (matches
`tools/engine-tex-compress/`'s shape: `Cargo.toml` + `src/main.rs`,
no library split). The binary:

- **Reads** a glTF file (`.gltf` or `.glb`) from `--input PATH`.
- **Writes** one `.emsh` file per mesh primitive and one `.emat`
  file per unique material to `--out DIR/`.
- **Reports** a JSON manifest on stdout that lists the emitted
  artefacts, their content hashes, and the source-glTF node index
  each came from. The asset pipeline (Phase 7+) ingests the
  manifest to register the assets into a pak.

### 2. Vendored parser: `gltf` 1.4 (MIT-or-Apache-2.0)

The `gltf` crate handles glTF 2.0 parse + buffer accessors. It is
the only mature option in the Rust ecosystem; the alternatives
(`tobj`, `russimp`) cover OBJ / native-Assimp respectively and do
not parse glTF.

The `gltf` crate is statically linked into the `engine-mesh-import`
binary. **No other engine crate may depend on `gltf`** — the editor,
the runtime, and the engine-asset library all consume EMSH/EMAT
through `engine_asset::mesh` and never the raw glTF.

A new CI grep guard rejects `gltf` outside `tools/engine-mesh-import/`,
the same shape as ADR-049's `wgpu::` boundary guard. Texture decoders
the glTF crate pulls in transitively (e.g. `image`, `png`, `jpeg-
decoder`) are also subprocess-only — the runtime never decodes PNG.

### 3. Sandbox shape

The binary inherits the same subprocess-isolation discipline as the
slangc and texture-compress wrappers (per ADR-019):

- **Cleared environment.** `Command::env_clear()` with
  `LANG=C.UTF-8` forwarded — applied by the editor's subprocess
  wrapper at spawn time.
- **Closed stdin.** `Stdio::null()`.
- **Absolute path resolution.** The editor (or build script)
  resolves the binary path once; subsequent spawns reuse it.
- **Wall-clock timeout.** 60 s default in the editor wrapper (glTF
  parse + texture decode + EMSH emit on a large scene model is
  realistic; the 30 s default from ADR-019 is too tight for big
  assets).
- **Subprocess boundary.** The binary itself is the isolation unit
  — a parser crash in `gltf` cannot crash the editor; the editor
  surfaces it as a typed exit code per §6.

In-process seccomp-bpf (Linux) / AppContainer (Windows) /
sandbox-exec (macOS) filtering — the syscall-restricted profile
ADR-019 §Decision describes — is a future addition. The
`engine_platform::sandbox` module that would host it does not exist
as of Phase 6 PR 1; adding it is a Phase 7+ work item with
engine-platform scope. Today the subprocess boundary is the security
property the editor relies on, matching the as-shipped behaviour of
the slangc and texture-compress wrappers.

### 4. CLI shape

Owned arg parser, no `clap`, matching `tools/engine-tex-compress/src/
main.rs`'s style:

```text
engine-mesh-import --input <PATH> --out <DIR>
                   [--material-shader <ID>]
                   [--coordinate-system y-up | z-up]
                   [--tangent-mode mikkt | gltf]
                   [--max-vertices-per-mesh N]
                   [--quiet | --verbose]
```

`--material-shader` is the Slang shader's BLAKE3 truncated `shader_id`
that emitted EMATs reference. Defaults to the engine's standard PBR
shader (`shaders/pbr_opaque.slang.bundle`) when the build script
runs the importer; required when invoked stand-alone.

`--coordinate-system` defaults to `y-up` (the renderer's convention;
matches `testbed/engine-raster::scene`); `z-up` is supported for
DCC-tool exports that use Blender's native frame.

`--tangent-mode mikkt` (default) computes tangents per MikkTSpace if
the glTF lacks them; `gltf` preserves whatever the source provided.
Determinism: MikkTSpace is the deterministic standard for tangents.

### 5. Output discipline

For every glTF mesh primitive:

- One EMSH file. Vertex attributes are filtered down to the
  semantics the engine recognises (per ADR-061 §1's
  `VertexSemantic` table). Joint weights are normalized to sum
  exactly 1.0 (renormalized per vertex if the source drift).
- A `SubMesh` per glTF primitive of the same mesh, sharing
  vertex/index buffers when source layouts match.

For every glTF material:

- One EMAT file. Texture references resolve to ETEX content hashes
  via a parallel invocation of `tools/engine-tex-compress/` —
  embedded glTF textures are extracted to a temp dir, run through
  the compressor, and their resulting ETEX hashes embedded in the
  EMAT slots. Scalar PBR factors (base color, metallic, roughness)
  populate the `factors[]` array in shader-reflection order.

Multiple primitives sharing a material produce one EMAT (content-
addressed dedupe is the pak's job, but the importer also dedupes
at emit time to avoid wasted compression work).

### 6. Error contract

- `ImporterError::ParserCrash` — the subprocess died abnormally
  (SIGSEGV, SIGABRT, abort, exceeded timeout). The editor harness
  surfaces a typed error; no editor crash.
- `ImporterError::SchemaInvalid` — glTF validation failed (missing
  required fields, malformed JSON). Surfaces as exit-code 2 +
  stderr message.
- `ImporterError::Unsupported` — glTF references a feature outside
  the v1 importer scope (e.g. KHR_materials_volume). Surfaces as
  exit-code 3 + stderr message naming the unsupported extension.
- All errors are typed in `engine_asset::ImporterError`; the
  editor consumes them via the subprocess wrapper's exit-code
  conventions (ADR-019 §Decision).

## Rationale

- **ADR-019 closes the security question.** glTF is exactly the
  threat model ADR-019 describes (mixed JSON + binary + embedded
  texture parse, with C-FFI texture libraries). Running the importer
  in-process would be the spec-violating shortcut.
- **`gltf` 1.4 is the only mature parser.** The crate has a stable
  API across years; its dependency tree (image, json) is well-known
  and CVE-tracked; static-linking it into a subprocess binary
  contains any future CVEs to that process boundary.
- **The CLI's discipline mirrors `tools/engine-shader/`.** One
  editor consumer; one CI consumer; the binary is reusable from
  scripts. Owned arg parser keeps the dependency surface flat.
- **Texture extraction reuses `tools/engine-tex-compress/`.** No new
  texture pipeline; the importer composes the existing tool. This
  is the same composition pattern the editor uses to call slangc
  through engine-shader's library.

## Consequences

- One new workspace member. `Cargo.toml` adds
  `tools/engine-mesh-import` to `members`. Adds `gltf = "1.4"` as
  a new direct dependency, used only by that binary.
- One new CI grep guard: reject `gltf::|use gltf\b` outside
  `tools/engine-mesh-import/`. Same shape as ADR-049's wgpu guard.
- Adds `image`, `png`, `jpeg-decoder`, and similar transitive deps
  to the workspace lockfile (via `gltf`). They are *subprocess-only*
  by guard; the runtime crates cannot import them.
- `engine_platform::sandbox` (when it lands; Phase 7+) will gain a
  mesh-import seccomp profile — one additional allowlist entry
  (write-syscalls on the output dir). Phase 6 PR 1 ships without it.
- `deny.toml` may need a license clarification entry for `gltf` and
  its transitive `image` (both MIT/Apache-2.0); confirmed
  permissive.
- `docs/architecture/engine-asset.md` is added in this PR to document
  the new mesh + material asset kinds and the importer subprocess
  (`engine-asset` was the largest crate without an architecture
  doc per Phase 5 follow-ups).

## Risks and tradeoffs

- **Subprocess startup latency (~10 ms Linux, ~30 ms Windows)** is
  amortized over per-mesh import: the build script invokes the
  importer once with a list of glTF files and produces all
  artefacts in one process pool batch.
- **glTF feature subset.** v1 importer rejects glTF features
  outside the engine's PBR surface (KHR_materials_iridescence,
  KHR_animation_pointer, etc.). Documented in the importer's `--help`
  output; extension support is a future ADR amendment + a PR.
- **Coordinate-system mismatches.** Blender exports z-up by default;
  the importer transforms to y-up unless `--coordinate-system z-up`
  is passed. A misconfigured export produces a deterministic but
  rotated mesh — visible in the oracle.
- **MikkTSpace tangent regeneration is non-trivial.** Adds ~50 ms
  per 10k-vertex mesh to import time. Acceptable for build-time;
  the alternative (passing through whatever the DCC tool produced)
  yields non-deterministic tangents across exporters.
- **Cross-platform sandbox differences.** Linux gets seccomp-bpf
  via `engine_platform::sandbox`; Windows gets AppContainer +
  job-object; macOS gets sandbox-exec. Same per-platform divergence
  ADR-019 already shoulders.

## Alternatives considered

- **In-process `gltf` crate import.** Smaller code; smaller import
  latency; loses the security property. Rejected by ADR-019.
- **WASM-sandboxed importer.** Strong sandbox; the `gltf` crate
  pulls in `image` which pulls in C deps that don't compile to
  WASM cleanly without alternatives. Phase 11+ candidate; not
  worth the porting cost now.
- **Skip glTF; require artists to author EMSH directly.** Loses
  every DCC tool. Rejected.
- **Vendor Assimp via FFI.** Larger format coverage (FBX, OBJ,
  COLLADA); much larger native dependency; FBX support requires
  a separate sandboxed subprocess for the proprietary library.
  Phase 11+ candidate.
- **A standalone editor plugin per DCC tool** (Blender add-on
  emitting EMSH directly). Useful long-term; complementary to
  glTF import; not Phase 6 scope.

## Verification

- Implementation lands in Phase 6 PR 1. Test files:
  - `tools/engine-mesh-import/tests/gltf_to_emsh_roundtrip.rs`:
    import a known glTF (a small bundled `cube.gltf`); assert the
    EMSH header fields match expected; reload via
    `engine_asset::mesh::MeshMeta::from_bytes` and compare vertex
    data byte-for-byte.
  - `tools/engine-mesh-import/tests/gltf_material_to_emat.rs`:
    import a glTF with a textured material; assert EMAT slot count,
    texture content hashes resolved against ETEX outputs.
  - `tools/engine-mesh-import/tests/gltf_red_team.rs`: a hand-
    crafted malformed `.gltf` known to crash `gltf` 1.4 in earlier
    versions; assert the editor harness reports
    `ImporterError::ParserCrash` and the editor process survives
    (ADR-019 §Verification).
- CI guard:
  - `.github/workflows/ci.yml` gate job gains a step:
    ```sh
    grep -rnE '\bgltf::|use gltf\b' crates bin tools testbed \
      | grep -v 'tools/engine-mesh-import/' \
      | grep -vE '^[^:]+:[0-9]+:[[:space:]]*//' \
      | grep -vE '\.md:'
    ```
    Non-empty match fails the job with
    `Route mesh import through tools/engine-mesh-import — see ADR-062`.
- Telemetry: the importer emits
  `SPAN "asset.import.gltf"` and `COUNTER "asset.import.gltf.bytes"`
  per ADR-010, consumed by the build script's pipeline-time
  reporter.
- The importer's own determinism oracle: identical glTF + identical
  flags → identical EMSH bytes (BLAKE3-hashed and compared in a
  test fixture).
