# ADR-090 — engine-savegame implementation (realises ADR-054)

- Status: Accepted (planning record; implementation lands in Phase 7 PR 8)
- Date: 2026-05-29
- Phase: 7 — PHYSICS + 2D (Engine Core v0.5)
- Companion: **ADR-054 (the contract this realises)**, ADR-008
  (content-addressed assets), ADR-012 (50-year stability), ADR-024
  (derive macros live in engine-ecs-macro), ADR-048 (BLAKE3 content
  addressing), ADR-057 (BLAKE3 primitive), ADR-082 (engine-config TOML
  reader), ADR-091 (the schema checker), spec §XIX.3

## Context

ADR-054 locked the save-game *contract* in the Phase-0 catchup and
declared it would be *realised* in Phase 7 ("This ADR closes (status:
Realised) when Phase 7 PR 1 lands"). Phase 7 is the first phase with
persistent game state to save (the platformer milestone). This ADR
records the implementation decisions that ADR-054 left to the
implementing PR, and **two refinements** to the contract.

## Decision

### 1. Crate layout (Level 2)

```
crates/engine-savegame/src/
  lib.rs            — SaveFile, save()/load(), public surface
  header.rs         — RON header parse/emit (ADR-054 field set)
  body.rs           — owned binary codec "engine-binary-v1" (no serde)
  migration.rs      — format-version chain (ADR-054 migrate_vN_to_vN+1)
  format_version.rs — CURRENT_VERSION const + version enum
```

Deps: `engine-core`, `engine-asset` (content-addressed body store),
`engine-reflect` (component field walk for the codec), `engine-config`
(ADR-082 — the RON-ish header reader; see refinement 2), `blake3`.

### 2. File format — `.sav` per ADR-054 §File format

RON header + binary body, exactly ADR-054's field set: `format_version`
(u32), `engine_version` (semver), `game_id`, `write_timestamp`
(RFC3339), `body_size` (u64), `body_hash` (see refinement 1),
`body_codec` ("engine-binary-v1"). Header signing stays **Phase 9+**
per ADR-054 (Ed25519 envelope for cloud-sync) — v0.5 ships the
hash-verified header, not a signed one. This is not a new deferral;
ADR-054 already placed signing in Phase 9+.

### 3. Migrations — format-version-level chain (ADR-054 §Migration)

The migration model is **ADR-054's chosen design**: a forward chain of
`fn migrate_v<N>_to_v<N+1>(SaveV<N>) -> Result<SaveV<N+1>>`, with
per-component logic *inside* each step. ADR-054 explicitly rejected a
per-component migration *registry*; this ADR honours that — the
implementation does **not** ship `register_migration::<Component, …>()`
(a phrasing the Phase-7 plan drifted toward). Each step is
deterministic and total; the schema-evolution policy (field add/remove
non-breaking; rename/type-change breaking) is ADR-054 §Schema
evolution verbatim.

### 4. The `Save` derive

`#[derive(Save)]` lives in `engine-ecs-macro` (per ADR-024 / ADR-054
§Consequences). It generates (a) the component's `engine-binary-v1`
encode/decode and (b) a **schema-descriptor row** (component name,
field names + types + version) that PR 9's `engine-savegame-check`
consumes. The descriptor is emitted to a generated
`component-schema.toml` (ADR-082 format) at build time.

### Refinement 1 — body hash is BLAKE3, not SHA-256

ADR-054 §File format wrote `body_hash_sha256`. The engine ships **no
SHA-256** anywhere; BLAKE3 is the workspace hash primitive (ADR-048
content addressing, ADR-057 RNG, every golden). Introducing `sha2`
solely for saves contradicts the minimal-dependency discipline
(ADR-051). **This ADR refines the field to `body_hash` carrying a
BLAKE3-256 digest**, recorded as an ADR-054 amendment at closure. The
content-address dedup key (ADR-054 §Cloud-sync) is likewise the BLAKE3
digest, consistent with ADR-008.

### Refinement 2 — header reader is engine-config

ADR-054 deferred the header parser's RON-crate choice ("the same crate
the future scene format uses; choice deferred"). The engine now owns a
line-oriented TOML reader (`engine-config`, ADR-082). The header is
emitted as the small, owned, debuggable key-value text `engine-config`
reads — no third-party RON crate enters the save reader. The header
stays human-inspectable, satisfying ADR-054's forward-debuggability
force.

## Rationale

- **Realise the contract, don't reinvent it.** ADR-054 did the design
  work; this ADR fills the two genuinely-deferred choices (hash
  primitive, header parser) with the engine's now-existing owned tools.
- **BLAKE3 over SHA-256** removes a dependency and matches every other
  digest in the tree.
- **engine-config header** keeps the save reader free of a new RON
  dependency while preserving debuggability.
- **Format-version migrations** are ADR-054's decision; the checker
  (ADR-091) enforces them at PR time.

## Consequences

- New Level-2 crate; `Cargo.toml` gains `engine-savegame`.
- `engine-ecs-macro` gains the `Save` derive + schema-descriptor
  emitter.
- ADR-054 gains a closure amendment (status → Realised; SHA-256 →
  BLAKE3; header parser → engine-config).
- PR 9 (`engine-savegame-check`) consumes the emitted
  `component-schema.toml`.

## Risks and tradeoffs

- **Owned binary codec bugs corrupt saves silently.** Mitigated by the
  round-trip + determinism oracles (ADR-054 §Verification) and
  malformed-input tests.
- **engine-config as header format** is TOML-ish, not RON. Accepted —
  it is owned, debuggable, and already in-tree; the ADR-054 force was
  "human-readable header," which TOML satisfies.

## Alternatives considered

- **SHA-256 per the original ADR-054 text.** Rejected — would add
  `sha2` for one use; BLAKE3 is the engine's hash.
- **Per-component migration registry.** Rejected — ADR-054 already
  rejected it; format-version chain stands.
- **A third-party RON crate for the header.** Rejected — engine-config
  is the owned reader (ADR-082).

## Verification

- `crates/engine-savegame/tests/roundtrip.rs` — encode→decode→entity-
  graph equal; write-read-write byte-identical (ADR-054 determinism).
- `crates/engine-savegame/tests/migrate.rs` — golden v1 save migrates
  v1→v2→CURRENT; state verified.
- `tests/fixtures/saves/` — committed golden vN saves.
- `engine-savegame-check` (ADR-091) gates schema evolution at PR time.
- `just ci` green at the PR-8 commit.
