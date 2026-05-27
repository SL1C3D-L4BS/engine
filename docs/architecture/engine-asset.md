# engine-asset

The content-addressed asset pipeline core (spec IV.1 Level 1, IV.8;
ADR-008, ADR-019, ADR-025, ADR-029, ADR-045, ADR-048, ADR-061, ADR-062).

## Purpose

`engine-asset` is the canonical home for **on-disk asset metadata** and
the **content-addressed pak loader**. Every compiled asset that the
engine reads at runtime — textures, meshes, materials, signed update
paks — flows through this crate. The crate's discipline:

- **Owned binary formats.** No serde, no third-party deserializer.
  Each asset kind ships with a hand-written little-endian codec + a
  fixed deterministic header magic (`ETEX` / `EMSH` / `EMAT`).
- **Content addressing.** Every blob is identified by the
  [`ContentHash`] (SHA-256) of its bytes. Identical bytes → identical
  hash → the pak deduplicates automatically.
- **Mmap-friendly layout.** Headers are fixed-size; payloads follow
  contiguously so the runtime loader (ADR-029) hands back a zero-copy
  `&[u8]` straight from the memory-mapped file.

The crate is Level 1 — no GPU types, no rendering, no scene graph. Its
sole runtime consumer is the asset loader; its sole authoring-time
consumer is the importer subprocess CLI for each format.

## Modules

| Module    | Contents |
|-----------|----------|
| `hash`    | `ContentHash` — SHA-256 content addressing, with a hex round-trip for manifests. |
| `store`   | `ContentStore` — a deduplicating blob store; identical bytes hash to one key and one stored copy. |
| `pak`     | `Pak` archives (deterministic serialized form, integrity-checked on decode), `PakSet` — newest-first overlay resolution with a per-name kill-switch (ADR-048), `Pak::open_mmap` (ADR-029). |
| `handle`  | `Handle<T>` typed handles and the `AssetServer` — load, dedup, ref-count, hot-reload (ADR-036). |
| `sign`    | `PakSigner` / `verify` — Ed25519 signing of pak archives (ADR-025). |
| `texture` | `TextureMeta` (`ETEX`) — 24-byte header for compressed BC blobs (ADR-045 §4). |
| `mesh`    | `MeshMeta` (`EMSH`) — 24-byte header for geometry + index + sub-mesh + AABB payload (ADR-061 §1). |
| `material`| `MaterialMeta` (`EMAT`) — 24-byte header for texture-slot + factor payload (ADR-061 §2). |

## Owned binary format discipline

Every shipping asset format follows the same recipe:

1. **4-byte ASCII magic.** Identifies the format kind. Re-numbering
   is a breaking change to the pak.
2. **`version: u16 LE`.** Bumped when the layout grows; `decode`
   rejects an unsupported version with `PakError::UnsupportedVersion`.
3. **`flags: u16 LE`.** Reserved bits, zero in v1. Future expansion
   reclaims them in-place; the v1 hash remains stable.
4. **Typed counts + sizes.** Each count is the minimum-width
   little-endian unsigned integer that fits the cap; oversized counts
   trigger `PakError::OutOfBounds` on decode.
5. **`payload_len: u32 LE`.** Total byte length of the typed payload
   that follows the header. Validated against the actual byte tail on
   `decode` — a mismatch is `PakError::OutOfBounds`.
6. **Reserved bytes are zero on encode** and **validated to be zero
   on decode** (`material`'s 6 reserved bytes, `texture_slot`'s 6
   reserved bytes). This preserves bit-for-bit hash stability across
   future v2 expansion.

The three implementations (`texture.rs`, `mesh.rs`, `material.rs`)
share the recipe; reading any of them is enough to read all three.

## EMSH payload layout (ADR-061 §1)

After the 24-byte `MeshMeta` header:

```text
EMSH payload (variable length = MeshMeta::expected_payload_len()):
  vertex_data    [u8; vertex_count * vertex_stride]
  index_data     [u8; index_count * (2 if U16 else 4)]
  sub_meshes     [SubMesh; sub_mesh_count]   // 12 bytes each
  aabb           [f32; 6] LE                   // min.xyz, max.xyz
```

`SemanticMask(u8)` drives per-vertex layout: for each set bit in
LSB→MSB order, the corresponding `VertexSemantic`'s bytes appear at
the vertex's stride offset. `vertex_stride` is the sum of present
semantics' byte sizes and is cross-validated on decode — a stride
mismatch is `PakError::Truncated`.

Sub-mesh records map index ranges to material indices for
multi-material meshes. The v1 cap is 16 sub-meshes per mesh; higher
counts trigger `PakError::OutOfBounds`.

## EMAT payload layout (ADR-061 §2)

After the 24-byte `MaterialMeta` header:

```text
EMAT payload (variable length = MaterialMeta::expected_payload_len()):
  texture_slots  [TextureSlot; texture_count]  // 40 bytes each
  factors        [f32; factor_count] LE         // shader-reflection order

TextureSlot (40 bytes):
  semantic         u8                            // TextureSemantic
  sampler_kind     u8                            // SamplerKind preset
  reserved         [u8; 6]  = [0; 6]
  content_hash     [u8; 32]                      // SHA-256 of bound ETEX
```

`shader_id` in the header is a truncated 32-bit prefix of the target
Slang `Bundle` digest (ADR-037 §Artefact format). The runtime looks
the full digest up in the pak when binding the material; a mismatch
between `shader_id` and the bundle's digest prefix is a hard load
error.

The v1 caps are 16 texture slots + 32 factors per material. The Phase
6 PR 1 importer (`tools/engine-mesh-import/`) emits `texture_count =
0` materials — texture extraction joins in a follow-up that
orchestrates `tools/engine-tex-compress/` to derive ETEX content
hashes from glTF's embedded images.

## Importer subprocesses (ADR-019 + ADR-062)

Source content lives outside the engine — DCC tools emit PNG / EXR /
KTX2 textures and glTF 2.0 meshes. The engine never parses those
formats in the runtime process. Per-format subprocess CLIs in
`tools/` convert source to owned binary artefacts:

| Format        | Tool                          | Output |
|---------------|-------------------------------|--------|
| RGBA8 image   | `tools/engine-tex-compress`   | ETEX (BC4/5/7-compressed) |
| Slang shader  | `tools/engine-shader`         | Bundle (`SHDR` per ADR-037) |
| glTF 2.0      | `tools/engine-mesh-import`    | EMSH + EMAT |

Each importer follows the ADR-019 subprocess discipline:

- **One binary per format.** The third-party parser (`intel_tex_2`,
  `slangc`, `gltf`) is statically linked into that binary alone. CI
  grep guards reject the parser's identifiers outside the importer's
  directory (ADR-049's wgpu guard, ADR-062's gltf guard,
  ADR-037's naga/shaderc reject).
- **Owned arg parser, owned JSON.** No `clap`, no `serde_json` — the
  CLIs are small enough to hand-roll, and the manifest output is a
  deterministic JSON byte stream consumable by the asset pipeline
  without third-party parsers.
- **Subprocess isolation.** The editor invokes the CLI as a separate
  process; a parser crash in the subprocess cannot crash the editor.
  Typed exit codes (`2` schema, `3` unsupported, `4` io, `5`
  parser-crash, `64` usage) discriminate failure modes.
- **No in-process seccomp filter yet.** ADR-019 §Decision describes a
  future `engine_platform::sandbox` module that would add seccomp-bpf
  filtering to every spawn. That module does not exist as of Phase 6
  PR 1; the editor relies on the process-boundary property alone for
  now. Adding seccomp is a Phase 7+ work item with engine-platform
  scope.

## Pak overlay + kill-switch (ADR-048)

`PakSet` stacks paks with newest-mounted-first resolution. Each mount
returns a handle whose drop unmounts the layer
(`PakSet::unmount_last`). A per-name kill-switch lets a hotfix pak
hide a base-pak asset without rebuilding the base. The
`dedupe_refcount_does_not_double_free` test (Phase 5 PR 6, ADR-048
§Verification) guards the unmount path.

## Design notes

- A `pak`'s serialized form is sorted and therefore byte-deterministic:
  the same inputs always produce the same archive, which is what makes
  signing and delta-patching meaningful.
- `AssetServer` hot-reload swaps the value inside a slot, so handles
  held since before the reload observe the new asset with no
  invalidation.
- Cryptography is delegated to audited crates, not owned (ADR-025).
- Every codec emits bytes in fixed little-endian order. The cross-arch
  determinism oracle (`.github/workflows/ci.yml` `determinism` job)
  verifies x86-64 + aarch64 produce identical hashes for identical
  input.

## Out of scope

- **FBX / OBJ / COLLADA / USD importers.** Phase 11+ per ADR-019; each
  ships as a sibling subprocess CLI on the same template.
- **In-process texture / image decoders.** PNG / JPEG / EXR decoding
  belongs in `tools/engine-tex-compress/`; the runtime sees only
  BC-compressed ETEX.
- **Scene / save-game formats.** `.scn` and `.sav` need the concrete
  component types that arrive in Phase 7+ (ADR-054).
- **Animation curves + skinning data.** `VertexSemantic::BoneWeights`
  and `BoneIndices` are defined in the EMSH format but not yet emitted
  by the glTF importer. Skinning lands with the animation system
  (Phase 7+).

## Oracle

- `tests/pipeline.rs` — content-address reproducibility and
  deduplication, pak sign → verify round-trip (and tamper detection),
  overlay newest-wins resolution with the kill-switch, end-to-end
  `AssetServer` hot-reload path.
- `tests/mmap_roundtrip.rs` — `Pak::open_mmap` zero-copy validation
  (ADR-029).
- `tests/emsh_emat_codec.rs` — EMSH + EMAT round-trip + content-hash
  stability + one-bit-flip discrimination (ADR-061 §Verification).
- `tools/engine-mesh-import/tests/gltf_to_emsh_roundtrip.rs` — the
  importer's smoke + determinism + red-team + typed-exit-code coverage
  (ADR-062 §Verification).

## Dependencies

`engine-platform`, `engine-core`, `sha2`, `ed25519-dalek` — Level 1.
The Phase-6 mesh + material codecs add no new crate dependencies; the
glTF parser is confined to `tools/engine-mesh-import/` by the
ADR-062 CI guard.
