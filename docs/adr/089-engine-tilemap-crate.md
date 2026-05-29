# ADR-089 — engine-tilemap Level-2 crate + `.tilemap` format

- Status: Accepted (planning record; implementation lands in Phase 7 PR 6)
- Date: 2026-05-29
- Phase: 7 — PHYSICS + 2D (Engine Core v0.5)
- Companion: ADR-061 (EMSH/EMAT owned-binary header pattern), ADR-078
  (ESPL 24-byte header + payload BLAKE3 — directly mirrored), ADR-062
  (glTF importer subprocess pattern), ADR-086 (2D physics collision
  surface), ADR-088 (tilemap.chunk render), ADR-019/ADR-098 (importer
  sandbox), spec §IV.431 / §IV.477 / §IV.511

## Context

Phase 7's portfolio (spec line 1640) includes **tilemap**. The spec
fixes the format map (§IV.477): `.tmx → .tilemap`, i.e. the source is a
Tiled XML map and the compiled runtime format is `.tilemap`. The
render path (§IV.431) is `tilemap.chunk` — "chunk-based, GPU
index-texture lookup". The AI path (§IV.511) aligns a NavGrid `u8`
walkability bitmap to the tilemap.

The runtime must never re-parse `.tmx`; like every other asset, the
import is an offline sandboxed subprocess (ADR-019/062) emitting an
owned, deterministic, content-addressed binary the engine memory-maps.
This mirrors `.mesh`/`.mat` (ADR-061) and `.espl` (ADR-078).

## Decision

### 1. Crate layout (Level 2)

```
crates/engine-tilemap/src/
  lib.rs       — Tilemap, TilemapChunk, public surface
  format.rs    — `.tilemap` 24-byte header + payload encode/decode
  chunk.rs     — 16×16 tile pages, SoA tile IDs + collision-rect SoA
  streaming.rs — load-on-demand chunk paging
```

Deps: `engine-asset` (mmap + content addressing), `engine-math`
(rects), `engine-render-2d` (chunk render). The collision-rect surface
is consumed by `engine-physics::r2d` (ADR-086).

### 2. `.tilemap` format — 24-byte deterministic header (ADR-078 pattern)

```
offset size field
0       4    magic         = b"ETMP"   (Engine TileMaP)
4       4    format_version : u32 LE
8       4    chunk_count    : u32 LE
12      2    tile_px_w      : u16 LE
14      2    tile_px_h      : u16 LE
16      4    flags          : u32 LE   (bit0 = has-collision, bit1 = streamed)
20      4    payload_digest : u32 LE   (low 4 bytes of BLAKE3(payload))
24      …    payload (chunk table + chunk pages, see §3)
```

The full BLAKE3 of the payload is recomputed on load and the low-32
checked against `payload_digest` (cheap tamper/corruption guard; the
content address is the full digest, ADR-048). All multibyte fields are
little-endian; encode is deterministic (no map iteration, no
floats) so the same source `.tmx` always yields byte-identical
`.tilemap` — the importer reproducibility golden (ADR-062 pattern).

### 3. Chunks — 16×16 pages, SoA

The map is partitioned into 16×16-tile chunks in **row-major chunk
order** (deterministic). Each chunk page is SoA: a `tile_id: u32`
array (0 = empty), and — when `flags.has-collision` — a
`collision_rects: [Rect]` SoA derived at import from solid tiles
(merged into maximal rectangles to keep the physics static-collider
count low). `streaming.rs` loads/evicts chunk pages on demand keyed by
chunk coordinate; the physics world adds/removes the chunk's static
colliders on the same event (ADR-086 §4).

### 4. Importer subprocess

`tools/engine-tilemap-import/` is a sandboxed CLI (ADR-019/062
pattern; seccomp-filtered per ADR-098) reading a Tiled `.tmx` (and, as
a convenience, a Tiled JSON export) and emitting `.tilemap`. The Tiled
XML/JSON parser is linked **only** into this tool; a CI grep guard
rejects the parser crate outside it (mirrors the `gltf::` guard).

### 5. NavGrid forward-compat

The collision bitmap is laid out so a future NavGrid (`u8` walkability,
spec §IV.511) can be derived 1:1 from the solid-tile mask without
re-deriving geometry. v0.5 ships the collision rects; the NavGrid is an
AI-phase consumer, noted here only so the layout does not foreclose it.

## Rationale

- **Mirror ESPL (ADR-078) exactly** — the 24-byte header + payload
  BLAKE3 is the engine's established owned-binary discipline; reusing
  it means the determinism + content-addressing tests are copy-adapt.
- **16×16 chunks** match the spec's chunk-based render and give the
  streaming + physics-collider granularity a platformer needs.
- **Collision rects merged at import** keep the runtime physics
  static-collider count small (one OBB per merged rect, not per tile).

## Consequences

- New Level-2 crate + new sandboxed importer tool; `Cargo.toml`
  `[workspace.dependencies]` gains `engine-tilemap`, and `members`
  gains `tools/engine-tilemap-import`.
- `engine-physics::r2d` gains `add_static_tilemap()` (ADR-086 §4).
- `engine-render-2d` renders chunks via the index-texture lookup path.
- A new CI grep guard for the Tiled parser crate.

## Risks and tradeoffs

- **Rect-merge is import-time work** with a determinism requirement.
  Mitigated by a fixed greedy-merge order (row-major) and the importer
  reproducibility golden.
- **Streaming churn** could thrash physics colliders at chunk
  boundaries. Mitigated by hysteresis (load a ring of neighbour chunks)
  — a streaming-policy constant, not a format concern.

## Alternatives considered

- **`.tmap` extension (original plan).** Rejected — spec §IV.477 fixes
  the compiled extension as `.tilemap`; the magic stays `ETMP`.
- **Per-tile collider (no merge).** Rejected — explodes the physics
  static-collider count on large solid regions.
- **Runtime `.tmx` parsing.** Rejected — violates the owned-asset +
  sandboxed-import discipline (ADR-019/061/062).

## Verification

- `crates/engine-tilemap/tests/format_roundtrip.rs` — encode→decode
  byte-identical; header digest check; deterministic re-encode.
- `crates/engine-tilemap/tests/chunk_collision.rs` — solid-tile mask →
  merged rects against a hand-checked fixture.
- `tilemap_collide_2d` physics replay fixture (ADR-086) consumes a real
  `.tilemap`.
- The platformer (ADR-100) renders + collides a streamed tilemap.
- `just ci` green at the PR-6 commit.
