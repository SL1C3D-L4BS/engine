# ADR-008 — Content-addressed asset pipeline

- Status: Accepted
- Date: 2026-05-18 (expanded 2026-05-24 per audit §15 Phase-0 ADR sweep)
- Phase: 0 (pre-v1.0 — contract-exempt per risk R-03)
- Companion: ADR-025 (audited crypto crates — sha2 is the hash),
  ADR-029 (mmap-backed pak loader — the runtime side),
  ADR-048 (pak overlay composition), ADR-052 (reproducible build
  cadence — depends on this property)

## Context

The engine's asset pipeline transforms source files (meshes, textures,
shaders, audio, scripts) into a runtime-loadable form (paks: packed
archives mapped via mmap, ADR-029). Three architectural choices have
to be made up front:

1. **What is the addressable unit?** A whole pak? A mesh inside a pak?
   A single primitive within a mesh? The grain determines patching,
   deduplication, and cache effectiveness.
2. **What is the address?** A path-and-version? A monotonic ID? A
   content hash?
3. **What is the trust boundary?** Source-of-truth in the user's
   editor (mutable) vs. shipped paks (immutable, signed,
   distributable)?

The literature converges (Bazel's CAS, Nix's content-addressed
store, Sapling's blob layer, Bagger): content-addressing — keyed
by cryptographic hash of the bytes — gives determinism,
deduplication, integrity, and patching for free.

## Decision

Every asset in the pipeline is keyed by the SHA-256 of its bytes.
The format:

```text
ContentHash := SHA-256(asset_bytes)        // 32 bytes
PakIndex    := { ContentHash → (offset, length, kind) }
Pak         := [header][index][asset blobs...]
```

The pak header includes a manifest hash, format version, optional
Ed25519 signature (ADR-025), and the offset table. Pak readers
verify the manifest hash on open; runtime asset access verifies
the content hash on first touch (mmap-friendly: the verification
happens during pre-warm, not on the hot path).

`engine-asset::ContentStore` is the runtime API: assets are
fetched by `ContentHash`; the store transparently demuxes across
pak files and overlay paks (ADR-048).

The address space is the SHA-256 image — 2^256 — which makes
collision the operational equivalent of impossible. Identical
bytes from different source files dedupe automatically.

## Rationale

Content-addressing is the foundation for several otherwise
independent engine properties:

- **Determinism.** A pak built from the same source produces
  the same hash, guaranteeing reproducible builds (ADR-052).
- **Deduplication.** A texture used by 10 materials lives once
  in the pak; the index has 10 entries pointing to the same
  blob.
- **Delta patching.** A Live Ops update ships only the new
  hashes; the runtime resolves them against the existing pak
  + overlay stack (ADR-048).
- **Integrity.** A corrupted byte changes the hash; the runtime
  detects corruption on first touch. With Ed25519 signing
  (ADR-025), the runtime also detects forgery.
- **Cache coherence.** The asset cache is keyed by hash; a hit
  is a hit, no version-skew accidents.

The pattern mirrors Bazel's CAS and Nix's store; both have
operated at petabyte scale for over a decade. The engine's
constraints are smaller and the approach is well-understood.

## Consequences

- The asset pipeline emits hashes deterministically. The
  pipeline itself must be deterministic — same input,
  byte-identical output — which is the reproducible-build
  property (ADR-052).
- `engine-asset` depends on `sha2` (ADR-025) and `ed25519-dalek`
  (ADR-025) — the two crypto dependencies the engine accepts.
- Runtime asset references are 32-byte hashes, not paths.
  Pretty names are stored separately in the editor-only
  manifest (the editor maps "models/player.gltf" to its hash
  on import; the runtime never sees the path).
- Pak overlay composition (ADR-048) becomes natural: an overlay
  pak's index is merged with the base pak's index by hash; a
  later overlay's hash for the same logical asset replaces the
  earlier one's, but both blobs remain on disk (older builds
  can still resolve their references).

## Risks and tradeoffs

- **SHA-256 is 32 bytes.** A reference is 4× the size of a
  u64 ID. Acceptable: most references are in cold structures
  (the runtime asset graph), not hot per-frame data.
- **Hash recomputation cost** on every asset build. Mitigated
  by the pipeline's incremental discipline (only re-hash
  inputs that changed) and by `sha2`'s SIMD path (the engine's
  hash cost is dominated by I/O, not by hashing).
- **No human-readable URIs at runtime.** Mitigation: the editor
  maps names to hashes; the `engine-tui` introspector (Phase
  10) surfaces "this hash was named models/player.gltf in
  build X."
- **Garbage collection of unreferenced blobs.** A pak's
  contents are addressed by hash but the live set is the
  transitive closure of root references. An unreferenced blob
  is dead weight; the pak compactor (Phase 10 tooling)
  reclaims them.

## Alternatives considered

- **Path-and-version addressing.** Standard engine pattern;
  loses deduplication and patching properties. Rejected.
- **Monotonic ID** (a per-build counter). Order-dependent;
  breaks reproducibility (two simultaneous build threads
  generating IDs collide). Rejected.
- **xxhash** instead of SHA-256. 8 bytes; vastly faster; no
  collision-resistance guarantee. Considered for the hot-path
  cache key only; rejected for the asset address (the integrity
  property is non-negotiable).
- **BLAKE3** instead of SHA-256. Faster than SHA-256; same
  collision-resistance. Considered; SHA-256 chosen for ecosystem
  ubiquity (every tool that ever reads the pak has SHA-256
  available; BLAKE3 still has gaps in 2026 outside the Rust
  ecosystem). The engine *does* use BLAKE3 internally for the
  determinism RNG (ADR-057) — different role, different cost
  profile.

## Verification

- `cargo test -p engine-asset` — round-trip pak format tests.
- The pak loader (`engine-asset::pak_loader`, ADR-029) verifies
  hashes on first access; corrupted blobs produce a typed
  error.
- The asset pipeline's deterministic emission is verified by the
  reproducible-build oracle (ADR-052) — same source produces
  byte-identical paks.
- Overlay composition (ADR-048) has its own oracle: stacking
  two paks with overlapping hashes resolves to the later one;
  hashes the later pak doesn't redefine resolve to the earlier
  one.
- The Live Ops delta-patch flow (Phase 10+) will inherit this
  ADR's properties by construction.
