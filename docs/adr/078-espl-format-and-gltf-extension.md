# ADR-078 — ESPL asset format + glTF KHR_gaussian_splatting reader

- Status: Accepted (Phase 6 design contract; implementation lands in
  Phase 6 PR 2)
- Date: 2026-05-28
- Phase: 6 — NEURAL RENDERING & GAUSSIAN SPLATTING
- Companion: ADR-061 (mesh + material owned formats — the precedent
  this format mirrors), ADR-062 (glTF importer subprocess pattern),
  ADR-008 (content-addressed asset pipeline), ADR-048 (pak overlay
  composition), ADR-077 (3DGS architecture — parent),
  ADR-084 (Phase 6 PR slicing)

## Context

ADR-061 established the engine's owned binary asset format for
meshes (`EMSH`) and materials (`EMAT`). Each is a fixed 24-byte
deterministic header followed by a content-addressed payload. The
importer subprocess pattern (ADR-062) consumes external formats
(glTF) in a sandboxed process and emits the owned binaries the
engine loads.

ADR-077's 3DGS deliverable needs the same treatment:

- **A loadable binary format** the engine can pak-stream without
  pulling a `.ply` parser into the per-frame load path.
- **An importer subprocess** that consumes the external interchange
  formats (`.ply` from the Kerbl reference; `.splat` from the
  INRIA-tooled interchange; eventually glTF's
  `KHR_gaussian_splatting` extension binary chunks).
- **A pak integration** so 3DGS clouds compose with other assets
  per ADR-048's overlay rules.

The 2025-ratification-track Khronos extension `KHR_gaussian_splatting`
defines a glTF + GLB layout for splat clouds: a `KHR_gaussian_splatting`
node holds an accessor reference to a single packed binary blob
containing the per-splat data. This is the long-term interchange
format the editor's import pipeline targets.

## Decision

### 1. ESPL binary format

24-byte deterministic header followed by an indexed payload:

```
Offset Size  Field
0      4     magic = "ESPL"
4      2     version (u16 little-endian; v1 = 1)
6      2     flags  (u16; bit 0 = has_sh; bit 1 = compressed (reserved); 2-15 reserved)
8      4     splat_count (u32)
12     4     payload_bytes (u32; total size of the data following the header)
16     8     BLAKE3 digest of payload (first 8 bytes; pak uses full 32-byte digest)
```

Total: 24 bytes. Identical structural shape to EMSH/EMAT per
ADR-061's "24-byte deterministic header" pattern. The `magic`
discriminates the kind in pak's resource directory.

### 2. Payload layout — SoA on disk

The on-disk payload is laid out section-by-section, each section
length-prefixed and tightly packed. This matches the in-memory
`SplatCloud` (ADR-077 §2) so decoding is a memcpy per section:

```
Section 0: positions   - N × 12 bytes (Vec3 f32 LE)
Section 1: scales      - N × 12 bytes (Vec3 f32 LE, log-space)
Section 2: rotations   - N × 16 bytes (Quat f32 LE, normalised)
Section 3: colors      - N × 12 bytes (Vec3 f32 LE)
Section 4: opacities   - N × 4  bytes (f32 LE, logistic-decoded)
Section 5: sh (opt.)   - N × 108 bytes (27 f32 LE)  -- present iff flag bit 0 set
```

No section-table indirection: a v1 file has sections in this fixed
order. Total bytes per splat without SH: 56. With SH: 164.

A 1M-splat scene without SH: ~56 MB.
A 1M-splat scene with SH: ~164 MB.

The flags byte gives forward-compatibility for quantisation (a
future v2 might add INT8-quantised SH at bit 2; v1 readers reject
unknown flag bits with a clear error).

### 3. Endianness + alignment

Little-endian throughout (matches every target). Each section
starts at an 8-byte-aligned offset (insert padding nulls if the
prior section is not 8-byte-aligned; the v1 sizes are all 4-byte
or larger so no padding is needed within a section, but the
position-after-header alignment to 8 means a 24-byte header is
8-aligned).

### 4. Asset trait implementation

```rust
// crates/engine-splatting/src/asset.rs
impl engine_asset::Asset for SplatCloud {
    const MAGIC: [u8; 4] = *b"ESPL";
    type Meta = SplatCloudMeta;

    fn encode(&self) -> Vec<u8> { /* writes header + 6 sections */ }
    fn decode(bytes: &[u8]) -> Result<Self, AssetError> {
        /* validates header, reads sections, builds SplatCloud */
    }
}

pub struct SplatCloudMeta {
    pub splat_count: u32,
    pub has_sh: bool,
    pub payload_digest: [u8; 32], // full BLAKE3 (not just truncated 8-byte header field)
}
```

Pak integration is automatic per ADR-008 / ADR-048: the encoder
emits the content-addressed payload; the loader memmaps the file
and decodes the in-place buffer (zero-copy where possible — the
positions / scales / rotations etc. are read directly into the
aligned vectors).

### 5. glTF KHR_gaussian_splatting reader

The glTF extension's binary layout differs from ESPL: glTF packs
the per-splat data into a single accessor with interleaved or
non-interleaved attribute streams. The reader code lives in
`crates/engine-splatting/src/gltf_ext.rs` *but is only invoked by
the importer subprocess*, never by the engine's per-frame loader.

The extension's draft (Aug 2025) names these attributes:

- `POSITION` (Vec3 f32)
- `_SCALE` (Vec3 f32, log-space)
- `_ROTATION` (Vec4 f32, normalised quaternion)
- `COLOR_0` (Vec3 or Vec4 f32; alpha = opacity)
- `_SPHERICAL_HARMONICS` (Vec27 f32, optional)

The reader maps these to ESPL's SoA layout. Mismatches between
draft revisions are handled by a single
`KHR_GAUSSIAN_SPLATTING_DRAFT_REV` const at the top of the reader
and a clear error on unsupported revs.

### 6. Importer subprocess interface

`tools/engine-splat-import/` accepts:

```
engine-splat-import \
    --in path/to/cloud.{ply,splat,glb} \
    --out target/cloud.espl \
    [--strip-sh]
```

The subprocess runs sandboxed (`engine_platform::sandbox::Sandbox`
per ADR-019) — no network, file access scoped to the input + output
directories, RLIMIT for memory and CPU. On success it writes the
ESPL binary + a JSON manifest (`cloud.espl.json`) recording the
source path, the format detected, and the BLAKE3 digest of the
output.

### 7. CI boundary guard

`.github/workflows/ci.yml` grows a grep guard rejecting `ply::` and
`splat_format::` usage outside `tools/engine-splat-import/`. Same
mechanism as ADR-062's glTF guard. This keeps the format-specific
parsers locked inside the sandboxed subprocess, never leaking into
the per-frame engine path.

## Consequences

### Positive

- The 3DGS asset story matches the existing mesh/material pattern;
  no new asset-pipeline abstraction.
- Pak integration is automatic — clouds participate in ADR-048's
  overlay composition like any other asset.
- The interchange formats (`.ply`, `.splat`, glTF) stay outside the
  engine binary's parse surface.
- The on-disk SoA layout is a memcpy decode; per-section copies
  preserve cache locality straight from disk to L2.

### Negative

- A 1M-splat cloud is 56–164 MB. Pak streaming + memmap remain
  appropriate, but the editor (Phase 10) will need a UX affordance
  for "very large asset" load times. Out-of-scope for Phase 6.
- The KHR_gaussian_splatting draft can revise before ratification;
  the reader is pinned to a specific draft rev with a clear error
  on mismatch. The importer subprocess is the regression-affecting
  surface; the engine never re-parses glTF at runtime.

### Neutral

- BLAKE3 digesting reuses the workspace's existing `blake3` dep —
  no new dependency.

## Implementation

PR 2 of Phase 6 (per ADR-084):

1. `crates/engine-splatting/src/asset.rs` — `SplatCloud::encode/decode`
   + `SplatCloudMeta`.
2. `crates/engine-splatting/src/gltf_ext.rs` — glTF reader.
3. `tools/engine-splat-import/` — subprocess CLI accepting
   `.ply`, `.splat`, and `.glb` inputs.
4. ESPL round-trip tests in `crates/engine-splatting/tests/asset.rs`.
5. CI grep guard for `ply::` + `splat_format::`.

## References

### Standards

- Khronos *KHR_gaussian_splatting* glTF extension (Draft, August 2025).
  <https://github.com/KhronosGroup/glTF/tree/main/extensions/2.0/Khronos/KHR_gaussian_splatting>.

### Interchange formats

- Original Kerbl et al. release `.ply` layout:
  <https://github.com/graphdeco-inria/gaussian-splatting/blob/main/scene/gaussian_model.py>.
- INRIA `.splat` interchange:
  <https://github.com/antimatter15/splat>.

### Prior engine ADRs

- [ADR-008](008-content-addressed-asset-pipeline.md) — the
  content-addressing discipline ESPL inherits.
- [ADR-048](048-pak-overlay-composition.md) — pak overlay rules
  ESPL participates in automatically.
- [ADR-061](061-mesh-material-owned-format.md) — the
  EMSH/EMAT pattern this format mirrors.
- [ADR-062](062-gltf-importer-subprocess.md) — the importer
  subprocess pattern this ADR replicates.
- [ADR-019](019-asset-sandbox-subprocesses.md) — the sandbox
  envelope `engine-splat-import` runs under.
- [ADR-077](077-3dgs-architecture.md) — the parent architecture
  ADR; `SplatCloud` is defined there.
- [ADR-084](084-phase-6-pr-slicing.md) — Phase 6 PR slicing.
