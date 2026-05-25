# ADR-054 — Save-game architecture

- Status: Accepted (contract; implementation lands when first
  persistent game state ships — Phase 7)
- Date: 2026-05-24
- Phase: cross-cutting (declared Phase 0 catchup; realised Phase 7)
- Companion: ADR-008 (content-addressed asset pipeline), ADR-012
  (50-year API stability contract), ADR-013 (determinism
  contract), spec §XIX.3 (save/load)

## Context

A 50-year game engine must let a player load a save from build N
in build N+1 (or N+50). The save format is the second-longest-
lived contract surface in the engine after `engine-api`'s
public types (ADR-012). The Phase-0 catchup PR locks the
contract before implementation begins so the engine's later
phases inherit the discipline.

The forces:

- **Backward compatibility** — a save file written today must
  open in every future engine version for at least the
  spec's 50-year support horizon (the same horizon as the
  API contract per ADR-012).
- **Forward debuggability** — when a save fails to load, a
  developer should be able to read the file structure
  (the header at least) without specialised tools.
- **Determinism** — a save round-trip (write → read → write)
  must produce byte-identical output. ECS state, RNG state,
  and any deterministic subsystem state are part of the save.
- **Footprint** — saves of large game worlds (1 M entities)
  should not require enterprise-class disk space; binary
  encoding for the body is required.
- **Migrations** — schema evolution is inevitable. The format
  must support migrating an older save to the current
  schema deterministically.

## Decision

### File format: `.sav`

```text
SavFile := SavHeader [body]
SavHeader := RON-text header { format_version, engine_version,
                              game_id, write_timestamp, body_size,
                              body_hash_sha256, body_codec }
[body]    := binary blob, codec specified in header
```

- **RON header.** Human-readable; engineers can inspect the
  header without engine. Field set: format_version (u32),
  engine_version (semver string), game_id (string),
  write_timestamp (RFC3339), body_size (u64), body_hash_sha256
  (32 bytes hex), body_codec (string enum, currently only
  "engine-binary-v1").
- **Binary body.** The body codec is owned (engine-binary-v1)
  — same compact little-endian binary discipline as the IPC
  body encoding (ADR-010 / ADR-051). No serde dependency.
- **Hash-verified.** The body's SHA-256 is in the header; the
  loader verifies on open. Corrupt saves fail typedly.
- **Optional encryption.** Phase 9+: an Ed25519 signature
  envelope (ADR-025) for save-cloud-sync; not part of this
  ADR's Phase-7 deliverable.

### Migration chain: `migrate_v<N>_to_v<N+1>`

For every save schema version N, the engine ships a
`fn migrate_v<N>_to_v<N+1>(save: SaveV<N>) -> SaveV<N+1>` in
`engine-savegame::migrations`. Opening an older save runs the
chain forward to the current version:

```rust
match save.header.format_version {
    1 => migrate_v1_to_v2(save).and_then(|s| migrate_v2_to_v3(s)).and_then(...),
    2 => migrate_v2_to_v3(save).and_then(...),
    // …
    CURRENT => Ok(save),
    v if v > CURRENT => Err(Future { v }),
    _ => unreachable!(),
}
```

Each migration is deterministic, total, and tested by a
migration-round-trip oracle: write a vN-format save with
known content, migrate to vCURRENT, load it, verify state.

### Schema evolution policy

- **Field addition** is non-breaking. A new field with a
  defaulted value can be added without a format-version bump
  (the migration codec defaults the field on load).
- **Field removal** is non-breaking. An obsolete field is
  ignored on load (the migration codec drops it).
- **Field rename or type change** is breaking. Requires a
  format-version bump + a `migrate_v<N>_to_v<N+1>`
  implementation.
- **Component addition / removal** in the ECS layer is a
  migration: a component the older save did not have is
  added with the type's `Default`; a component the older
  save has but the current engine doesn't is dropped (and
  logged at load time).
- **Deprecation window**: a deprecated component is supported
  for at least one full major engine version before
  migration code can be removed. Old saves more than two
  major versions back are an explicit "upgrade through an
  intermediate engine" path, not a single-step migration.

### Cloud-sync surface

- `engine-cloud-saves` plugin trait surface sketched in this
  ADR; concrete provider implementations (Steam Cloud, GOG
  Galaxy, EGS, custom CDN) ship as plugins per ADR-018.
- The save's body hash is the cloud-side dedup key — same
  pattern as ADR-008's content addressing applied to saves.
- Conflict resolution (player saves on machine A and machine
  B before sync) is a per-game policy. Engine ships
  "last-write-wins" and "manual-resolve" templates.

## Rationale

The RON header + binary body split is the same pattern the
engine uses for shader paks (ADR-037) and the asset pak
(ADR-008): debuggable header, efficient body. The cost (RON
parser dependency in the save reader) is acceptable — RON
parsing is cheap and the header is small.

The owned binary body encoding (vs MessagePack or serde-
generated) follows ADR-051's pattern: the save body is the
engine's own data, the encoder is small (~200 LOC), the
decoder is small. External tools that want to inspect a save's
body use the engine's decoder library (not parse the bytes
by hand).

The migration chain is a literature-standard pattern (Rails
migrations, Diesel migrations). The `migrate_vN_to_vN+1`
discipline keeps each step small and testable.

The schema-evolution policy is the load-bearing contract.
"Field addition is non-breaking" is the daily case; without
this guarantee, every PR that adds a component to a save
would require a version bump. The "default-on-load, drop-on-
read" rule for non-breaking changes keeps the daily case
cheap.

## Consequences

- `engine-savegame` is a new Level-2 crate (sketched in this
  ADR; landing with Phase 7's first persistent-state PR).
- The save body codec depends on no third-party crates (owned
  binary encoder pattern). The header parser depends on
  whatever RON crate the engine eventually settles on (the
  same crate the future scene format uses; choice deferred).
- Every component that ships in a save (every `#[derive(Save)]`
  component, in the future) gets a per-version migration
  story. The proc-macro derive (which goes in
  `engine-ecs-macro` per ADR-024) generates the per-version
  scaffolding.
- The save format is in the contract scope of ADR-012. Format
  changes go through PR review with the same semver
  discipline as engine-api changes.
- The cloud-sync trait surface ships in `engine-api` when the
  first provider lands (Phase 9+).
- The schema-evolution policy applies to all engine-controlled
  components. Game-controlled components inherit the
  discipline by convention; the migration story for them is
  the game shipper's responsibility.

## Risks and tradeoffs

- **Migration chain length.** After many engine versions, the
  chain to migrate an ancient save through N steps could be
  costly. Mitigation: the cost is paid once on save load; if
  it ever exceeds practical thresholds, an aggregated
  "skip-to-current" migration can be added per format version.
- **Schema-evolution policy violations.** A PR author might
  forget the policy and break a save format silently.
  Mitigation: a `cargo-semver-checks`-equivalent for save
  formats (Phase 7+ tooling — `engine-savegame-check`)
  catches breaking changes at PR time.
- **Owned encoder bugs.** A bug in the binary encoder corrupts
  saves silently. Mitigation: encoder round-trip oracle;
  fuzz testing of malformed inputs.
- **Per-game policy fragmentation.** Cloud-sync conflict
  resolution is game-specific; templates can drift. Mitigation:
  the engine ships one canonical conflict-resolution path
  (last-write-wins); games override only if they need
  custom logic.

## Alternatives considered

- **Serde-based binary** (`bincode` or similar). Pulls a
  significant transitive dep tree; owns the foundation-layer-
  deviation pattern (ADR-051). Rejected for the same reason
  the IPC body is owned.
- **Plain RON for everything** (header and body). Slow for
  large worlds; bloated. Rejected.
- **Plain JSON.** Same issues as RON, no advantages. Rejected.
- **SQLite-as-save.** Embedded database; significant
  dependency; the save is structured ECS data, not relational
  data. Rejected.
- **No migration support.** Forces players to start new games
  on every engine update; unacceptable for a 50-year engine.
- **Per-component migration registry** (vs. format-version-
  level migrations). More fine-grained; harder to reason about
  composition (component A's vN→vN+1 might depend on
  component B's vN). Rejected for format-version migrations
  with per-component logic *inside* the migration step.

## Verification

- Phase 7 ships `engine-savegame` with:
  - `tests/header_roundtrip.rs` — header parser oracle.
  - `tests/body_codec.rs` — body encoder/decoder oracle.
  - `tests/migration_chain.rs` — full vN→vCURRENT chain for
    every previously-shipped format version.
  - `tests/determinism.rs` — write-read-write is byte-identical.
- The `migrate_v<N>_to_v<N+1>` functions have unit tests with
  golden vN saves committed in `tests/fixtures/saves/`.
- `engine-savegame-check` tool (Phase 7+) verifies schema
  evolution at PR time.
- Cloud-sync trait surface goes through `cargo-semver-checks`
  via `engine-api`.
- This ADR closes (status: Realised) when Phase 7 PR 1 lands.
  Until then, it is the contract for the future implementation.
