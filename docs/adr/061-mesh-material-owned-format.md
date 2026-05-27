# ADR-061 — Owned mesh + material binary format (EMSH + EMAT)

- Status: Accepted (Phase 6 design contract; implementation lands in
  Phase 6 PR 1)
- Date: 2026-05-27
- Phase: 6 — RENDERING FOUNDATION (Track A, Part 2)
- Companion: ADR-008 (content-addressed asset pipeline), ADR-019
  (asset sandbox subprocesses), ADR-029 (mmap'd asset loader),
  ADR-044 (bindless heap), ADR-045 (texture compression — the ETEX
  precedent), ADR-062 (glTF importer), ADR-068 (Phase 6 PR slicing)

## Context

Phase 5 closed the renderer's trait surface (ADR-039) and the
deferred pipeline (ADRs 040–043). The renderer can already schedule
passes, compose a G-buffer, accumulate lighting, and run a post-FX
chain. It cannot, however, render a real model: every fixture in
`testbed/engine-raster/src/sample.rs` is procedural
(`combined_deferred_scene`, `cluster_lights_scene`,
`shadow_heavy_scene`). The engine has no asset format for mesh
geometry or for material parameter packs.

`engine-asset` does have one production owned format — `TextureMeta`
in `crates/engine-asset/src/texture.rs` (24-byte deterministic
header, magic `ETEX`, per ADR-045 §4). The pattern is established and
the discipline (no serde, hand-written little-endian codec, BLAKE3
content addressing per ADR-008) is the one Phase 6 must extend.

The gap is "what bytes does `engine-asset` hand the renderer when it
loads a mesh?" The answer must be an *owned* binary format because:

1. The mmap'd pak loader (ADR-029) requires a fixed, byte-deterministic
   layout for zero-copy access from a memory-mapped region.
2. The content-addressed asset pipeline (ADR-008) requires a layout
   stable across builds — adding a serde-derived bincode field
   would silently change the hash whenever serde renumbers fields.
3. The 50-year API contract (ADR-012) requires the on-disk format to
   be deliberately versioned, not implicitly tied to a third-party
   crate's evolution.

The vendored alternatives (`gltf` runtime, `meshopt`, `wgpu_glyph` style
asset crates) all violate one or more of those properties. Phase 6
ships owned formats and contains the vendor parser to the *import*
side (ADR-062).

## Decision

### 1. EMSH — mesh asset format

Magic `b"EMSH"`. 24-byte deterministic header followed by typed
payload sections. Encoded little-endian throughout, matching the
ETEX convention in `crates/engine-asset/src/texture.rs`.

```text
EMSH header (24 bytes):
  magic            [u8; 4]   = b"EMSH"
  version          u16 LE    = 1
  flags            u16 LE             // reserved bits; 0 in v1
  vertex_count     u32 LE
  index_count      u32 LE
  vertex_stride    u8                  // bytes per vertex
  index_format     u8                  // 0 = U16, 1 = U32
  semantic_mask    u8                  // bitset over VertexSemantic
  sub_mesh_count   u8                  // ≤ 16
  reserved         [u8; 4]  = [0; 4]   // pad to 24

EMSH payload:
  vertex_data      [u8; vertex_count * vertex_stride]
  index_data       [u8; index_count * (2 if U16 else 4)]
  sub_meshes       [SubMesh; sub_mesh_count]   // 12 bytes each
  aabb             [f32; 6] LE                  // min.xyz, max.xyz

SubMesh (12 bytes):
  first_index      u32 LE
  index_count      u32 LE
  material_index   u16 LE              // index into the pak's EMAT records
  flags            u16 LE              // reserved
```

`VertexSemantic` enum (`engine_asset::mesh::VertexSemantic`):

| Bit | Variant          | Layout                                |
|-----|------------------|---------------------------------------|
| 0   | Position         | 3 × f32 LE (12 B)                     |
| 1   | Normal           | 3 × f32 LE (12 B)                     |
| 2   | Tangent          | 4 × f32 LE (16 B) — w sign bit        |
| 3   | Uv0              | 2 × f32 LE (8 B)                      |
| 4   | Uv1              | 2 × f32 LE (8 B)                      |
| 5   | Color0           | 4 × u8 (4 B)   — sRGB                 |
| 6   | BoneWeights      | 4 × u8 normalized (4 B)               |
| 7   | BoneIndices      | 4 × u8 (4 B)                          |

`vertex_stride` is the sum of selected semantics' sizes. The importer
emits semantics in bit-order (Position → Normal → … → BoneIndices) so
runtime layout is fully determined by `semantic_mask`. A semantic
absent from the mask is absent from the per-vertex stride; the
renderer pads with a default value at sample time.

### 2. EMAT — material asset format

Magic `b"EMAT"`. 24-byte deterministic header followed by texture
slots and scalar factors. Material parameters are described by the
target shader's reflection (ADR-037); EMAT is an opaque pack the
shader's pipeline-layout consumes.

```text
EMAT header (24 bytes):
  magic            [u8; 4]   = b"EMAT"
  version          u16 LE    = 1
  flags            u16 LE
  shader_id        u32 LE             // truncated BLAKE3 of Slang Bundle
  texture_count    u8                 // ≤ 16
  factor_count     u8                 // ≤ 32 (≤ 128 B of scalars)
  reserved         [u8; 10] = [0; 10]

EMAT payload:
  texture_slots    [TextureSlot; texture_count]   // 40 bytes each
  factors          [f32; factor_count] LE          // shader-reflection order

TextureSlot (40 bytes):
  semantic         u8                              // ChannelRole (ADR-045)
  sampler_kind     u8                              // 0=Linear,1=Anisotropic,2=Point
  reserved         [u8; 6]  = [0; 6]
  content_hash     [u8; 32]                        // BLAKE3, references ETEX in pak
```

`shader_id` is the truncated 32-bit BLAKE3 prefix of the target Slang
`Bundle` digest from ADR-037. The runtime looks the full digest up in
the pak; a mismatch is a hard load error (the EMAT was built against
a different shader version).

### 3. Crate placement and Cargo surface

- `crates/engine-asset/src/mesh.rs` (new): `MeshMeta` + `VertexSemantic`
  + `SubMesh` + `Mesh` (owned-data variant for tests) + the encode /
  decode functions. Mirrors `texture.rs`'s shape one-to-one.
- `crates/engine-asset/src/material.rs` (new): `MaterialMeta` +
  `TextureSlot` + `SamplerKind` + the encode / decode functions.
  Mirrors `texture.rs` and reuses `engine_asset::hash::ContentHash`
  for the `content_hash` field.
- `crates/engine-asset/src/lib.rs`: `pub use {mesh, material}::*` —
  no new dependencies. The crate's `Cargo.toml` already pulls only
  `engine-platform`, `engine-core`, `blake3`; this PR adds none.

### 4. Hash-content-address discipline

Both formats are content-addressed via the existing
`engine_asset::ContentHash` type (BLAKE3 over the deterministic byte
stream). Same property as ETEX: identical content ⇒ identical hash
⇒ the asset pipeline dedupes in the pak.

The header's `reserved` bytes are *required* to be zero on encode
and *checked* to be zero on decode — non-zero reserved bytes are
`DecodeError::ReservedBytesNonZero`. This preserves bit-for-bit
hash stability when future versions add fields by reclaiming
reserved bytes.

## Rationale

- **ETEX is the precedent that works.** It ships, the runtime mmap'd
  loader (ADR-029) reads it zero-copy, the importer subprocess
  (`tools/engine-tex-compress/`, ADR-019 pattern) feeds the pak.
  EMSH + EMAT mirror its discipline byte-for-byte.
- **The 24-byte header is the unit that mmap-aligns cleanly.** Cache
  line is 64 B on every architecture we target; 24 B leaves 40 B of
  the line free for the first payload section without straddling.
- **`shader_id` as a truncated BLAKE3 is enough disambiguation.** Two
  shaders colliding on 32 bits is 1-in-4-billion; the full-hash lookup
  in the pak catches the collision case as a hard load error.
- **A bitset `semantic_mask` is cheaper than a per-attribute table.**
  Each vertex has at most 8 semantics; a 1-byte mask + ordered layout
  is exactly the surface the GPU vertex-buffer binding wants.
- **No `serde`.** The crate is foundation layer (Level 1); adding
  serde for two data records would pull in derive-heavy generics and
  break the owned-discipline established by ADRs 028, 029, 036, 045.

## Consequences

- `engine-asset`'s public surface gains two new types (`MeshMeta`,
  `MaterialMeta`) and two enums (`VertexSemantic`, `SamplerKind`).
  No removals; the texture surface is unchanged.
- The pak format (ADR-008, ADR-029) gains two new asset kinds. The
  Pak metadata table already addresses assets by content hash; adding
  EMSH/EMAT requires no header change to the pak itself.
- The renderer (Phase 6 PR 3 onward) consumes `MeshMeta` to drive
  `engine_gpu::Buffer` uploads for vertex + index data, and
  `MaterialMeta` to populate `engine_gpu::BindlessHeap` slots
  (ADR-044) for textures.
- The CPU oracle (`testbed/engine-raster`) gains a parallel
  loader so existing oracle fixtures can be re-emitted as EMSH +
  EMAT for the CPU↔GPU parity tests in PR 3 / PR 4.

## Risks and tradeoffs

- **The vertex layout is fixed to the bitset's bit order.** A future
  semantic added in a new bit slot is a compatible extension; reordering
  existing bits is not. Documented as a hard invariant in the
  `VertexSemantic` enum's doc-comment.
- **The 16-sub-mesh, 16-texture-slot, 32-factor caps** are pragmatic.
  Higher counts trigger `EncodeError::CapacityExceeded`. The caps can
  grow via a v2 format that consumes reserved bytes; an ADR amendment
  here records the change.
- **The `shader_id` truncation** means EMAT files alone cannot verify
  which Slang artefact they target. The pak's content-hash table is
  the source of truth; standalone EMAT inspection requires a Slang
  artefact alongside.
- **No quantization** in v1 (positions are f32, indices are U16/U32).
  Mesh quantization (e.g. 16-bit positions in an AABB-normalized
  space) is a Phase 11+ mobile optimization; the v1 layout is
  desktop-friendly.
- **No LODs in v1.** A mesh ships at one resolution; LODs are
  separate `EMSH` assets sharing a `material_index`. Multi-LOD
  packing is a v2 extension if measurements justify it.

## Alternatives considered

- **Vendored runtime glTF.** Inherits glTF's evolving JSON+binary
  layout; the runtime would parse JSON on every load. Rejected —
  violates owned-discipline and the mmap'd zero-copy property of
  ADR-029.
- **bincode + serde + a Rust struct.** Concise authoring; loses
  layout stability across serde versions; layout is `repr(Rust)`
  which the compiler may reorder; cannot inspect the bytes without
  a Rust deserializer. Rejected.
- **One unified `EMSH_v1` that embeds material inline.** Couples
  mesh and material; loses sharing (a mesh used with two materials
  duplicates the geometry). Rejected.
- **Flatbuffers / Cap'n Proto.** Zero-copy schemas; adds a heavy
  build-time codegen step; the schema language is third-party.
  Rejected — same R-02 stance as serde.

## Verification

- Implementation lands in Phase 6 PR 1. Test files:
  - `crates/engine-asset/src/mesh.rs` doc-tests for header round-trip.
  - `crates/engine-asset/src/material.rs` doc-tests for header
    round-trip.
  - `crates/engine-asset/tests/emsh_emat_codec.rs`: encode → mmap
    decode → field-by-field equality across {U16, U32} indices and
    {minimal, full} semantic masks.
  - `crates/engine-asset/tests/emsh_content_addressed.rs`: identical
    mesh content produces identical `ContentHash`; one-bit edit
    changes the hash.
  - `crates/engine-asset/tests/emsh_reserved_bytes_rejected.rs`:
    decode rejects non-zero reserved bytes.
- The CI determinism oracle (`.github/workflows/ci.yml` `determinism`
  job) runs the codec tests on x86-64 + aarch64; identical hashes
  required.
- No new CI guard needed; the ADR-028 grep guard (no
  `std::collections::HashMap` in `engine-asset/`) and the existing
  asset-layer discipline cover the new modules.
- PR 1 also lands a fixture: `testbed/engine-raster/fixtures/cube.emsh`
  plus a matching `.emat`; the oracle's `combined_deferred_scene`
  gains an opt-in variant that loads from the fixture.
