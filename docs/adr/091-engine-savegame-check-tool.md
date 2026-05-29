# ADR-091 — engine-savegame-check CLI + PR-time schema gate

- Status: Accepted (planning record; implementation lands in Phase 7 PR 9)
- Date: 2026-05-29
- Phase: 7 — PHYSICS + 2D (Engine Core v0.5)
- Companion: ADR-054 (§Risks names this tool), ADR-090 (the schema
  descriptor this consumes), ADR-082 (engine-config TOML), ADR-012
  (50-year stability), spec §XIX.3

## Context

ADR-054 §Risks identified the load-bearing failure mode: "A PR author
might forget the [schema-evolution] policy and break a save format
silently. Mitigation: a `cargo-semver-checks`-equivalent for save
formats (`engine-savegame-check`) catches breaking changes at PR time."
This ADR specifies that tool.

## Decision

### 1. The tool

`tools/engine-savegame-check/` is a CLI that:
1. Reads the **blessed baseline** schema descriptor committed at
   `crates/engine-savegame/schema/component-schema.toml`.
2. Regenerates the **working-tree** descriptor (the `Save` derive's
   emitter, ADR-090 §4, run over the current source).
3. Diffs the two per ADR-054's schema-evolution policy:
   - field addition / removal → **non-breaking** (allowed silently);
   - field rename / type change / component removal → **breaking**.
4. For every breaking change, requires both (a) a `CURRENT_VERSION`
   bump in `format_version.rs` and (b) a `migrate_v<N>_to_v<N+1>`
   covering the changed component. Missing either → **exit 1** with a
   diagnostic naming the component + the missing migration.
5. On a clean (non-breaking, or properly-migrated) diff → exit 0, and
   prints the command to re-bless the baseline
   (`engine-savegame-check --bless`).

### 2. CI gate

A new GitHub Actions job `savegame-schema-check` runs the tool on every
PR. It is **required** (blocks merge), mirroring the `wgpu::`/`gltf::`
boundary guards' enforcement posture. The job is fast (descriptor diff,
no build of the whole workspace beyond engine-savegame + its derive).

### 3. Descriptor format

The descriptor is the owned `engine-config` TOML (ADR-082), one
`[component.<Name>]` table per `#[derive(Save)]` type, with sorted
`fields = [...]` (name + type + since-version). Emit is deterministic
(sorted component + field order) so the diff is stable and the
re-bless step produces byte-identical output.

## Rationale

- **PR-time, not load-time.** A broken save format must be caught
  before it ships, not when a player's save fails to load years later.
- **Blessed baseline in-tree** makes the contract reviewable: the
  `component-schema.toml` diff appears in the PR, so a reviewer sees
  the schema change alongside the migration.
- **Reuse engine-config** — no new descriptor format.

## Consequences

- New tool; `Cargo.toml` `members` gains `tools/engine-savegame-check`.
- New required CI job + a committed `component-schema.toml` baseline.
- `engine-ecs-macro`'s `Save` derive (ADR-090) must expose the emitter
  as a callable the tool can drive (a small `--emit-schema` mode on a
  test binary, or a build-script artefact the tool reads).

## Risks and tradeoffs

- **Baseline drift** if a PR forgets to `--bless`. Mitigated — the tool
  exits non-zero on any descriptor mismatch that isn't a properly
  migrated breaking change, so the PR fails until re-blessed.
- **Generated-descriptor reproducibility** is required for a stable
  diff. Mitigated by deterministic sorted emit + a descriptor
  round-trip test.

## Alternatives considered

- **Runtime-only validation** (fail on load). Rejected — too late.
- **Manual review only.** Rejected — exactly the silent-break ADR-054
  warned about.
- **A general semver tool.** Rejected — save schemas are not Rust
  semver; the policy is ADR-054-specific.

## Verification

- `tools/engine-savegame-check/tests/` — a fixture with a non-breaking
  change passes; a fixture with an unmigrated breaking change fails
  (exit 1); a properly-migrated breaking change passes.
- The `savegame-schema-check` CI job is green on the PR-9 branch.
- `just ci` green at the PR-9 commit.
