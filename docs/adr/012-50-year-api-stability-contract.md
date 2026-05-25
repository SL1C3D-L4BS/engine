# ADR-012 — 50-year API stability contract

- Status: Accepted
- Date: 2026-05-18 (expanded 2026-05-24 per audit §15 Phase-0 ADR sweep)
- Phase: 0 (pre-v1.0 — contract-exempt per risk R-03)
- Companion: ADR-050 (engine-api activation — operationalises this
  contract), ADR-052 (reproducible build cadence — reproducibility
  is the verification mechanism), spec §XX.1

## Context

The engine targets a 50-year operational lifetime. Engines that
have not lasted (Source 1, UE3, Unity's pre-2017 builds) failed
not on language or platform technology but on accumulated API
debt — the public surface grew without discipline, dependents
took transitive bets, and breaking the API broke too many
downstream consumers to risk.

The architectural answer: a *single contract-bearing crate*. All
other crates are implementation; only one crate's public surface
is the contract a downstream binds against. Breaking changes
inside the implementation crates are private; breaking changes
to the contract crate are catastrophic events the engine
deliberately avoids.

## Decision

`engine-api` is the contract-bearing crate. Its surface is:

- A re-export of types and traits from the implementation crates
  (`engine_core`, `engine_render`, `engine_script`, `engine_asset`,
  `engine_net`, `engine_audio`, `engine_ai`, `engine_ui`, …) that
  downstream consumers should bind against.
- A type-stability pact: every type / trait / function exported
  from `engine-api` is covered by `cargo-semver-checks`.
- A deprecation policy: a deprecated item is marked
  `#[deprecated(since = "X.Y.Z", note = "...")]` for at least one
  full major version before removal. The minimum support window
  is two years.
- Data-format migrations: file formats produced or consumed by
  the engine (paks per ADR-008, save games per ADR-054, shader
  paks per ADR-037) carry version stamps and migration functions
  that read older versions into the current schema.

`engine-api` is enforced by CI:

- `cargo-semver-checks` runs on every PR against the previous
  release tag; a `Major` or `Minor` breaking change blocks the
  PR until rationale + version bump are explicit.
- A backwards-compat integration test suite consumes the
  *previous* engine-api version's published types and verifies
  it builds.

The contract is *only* `engine-api`. Internal crates
(`engine_core`, etc.) reserve the right to break their internal
surface freely; the bin/template/starter-kit grep guard
(Phase-0 catchup ADR-050 addendum, PR 0 Commit G) prevents bins
from importing internal crates and thereby silently turning them
into contract surface.

## Rationale

A single contract crate has three benefits:

1. **The blast radius of a breaking change is bounded.** Refactor
   `engine_render`'s internal traits freely; no downstream
   notices. Modify a re-export in `engine-api`, and CI fails
   with a clear "you broke the contract" signal.
2. **Semver becomes meaningful.** Without a contract crate,
   "the engine bumped major" is a soup of "what actually
   changed?" With the contract, the engine-api semver bump
   describes the actual user-observable change.
3. **50 years of compatibility windows.** Two-year minimum
   deprecation window × many cycles = downstream consumers can
   migrate at their own pace. Aggressive deprecation cycles
   (the Unity model) is the failure pattern this ADR avoids.

`cargo-semver-checks` is mature; landed in CI as a required gate
during the audit-remediation phase (the workflow's gate job
calls it on every PR).

## Consequences

- `engine-api` is initially empty (no public re-exports until
  Phase 10+). The grep guard's role between Phase 0 and Phase 10
  is "no internal crates may be imported from bin/templates/
  starter-kits" — same effect via different mechanism.
- Internal crates (`engine_core`, `engine_render`, etc.) are free
  to break their public surface; only `engine-api` is the
  contract.
- A new public surface lands by being re-exported from
  `engine-api`. Once re-exported, semver-checks owns it.
- Data-format migrations are themselves contract surface (an
  older save can be opened by a newer engine via the migration
  chain). ADR-054 fully specifies the save migration; the pak
  format (ADR-008) has implicit version-and-migrate properties
  via content-addressing.

## Risks and tradeoffs

- **The single-crate contract makes the engine-api a chokepoint.**
  Bad coupling between re-exports and implementation can
  proliferate. Mitigation: re-exports are simple; the audit
  reviews `engine-api`'s shape periodically.
- **cargo-semver-checks false positives.** The tool occasionally
  flags non-breaking changes as breaking. Mitigation: each false
  positive carries a code-review explanation; the maintainers
  upstream the bug report.
- **Long deprecation windows slow evolution.** Two-year minimum
  is a hard floor. Mitigation: aggressive use of additive
  evolution (new trait method with default impl is non-breaking;
  new struct field with default is non-breaking in many cases).
- **The "engine-api is empty until Phase 10" interim period**
  means the contract is theoretical for Phases 0–9. Mitigation:
  the grep guard substitutes; the contract activates the moment
  the first re-export lands.

## Alternatives considered

- **No contract crate; every internal crate is contract.** What
  Unity historically did; the source of the API debt. Rejected.
- **A contract crate per major area** (`engine-api-render`,
  `engine-api-net`, …). More flexible; semver-check coordination
  across multiple contract crates is harder; consumer dep graph
  fans out. Rejected.
- **No semver-checks; rely on code review.** Code review misses
  things; semver-checks is mechanical. Rejected.
- **Different time horizon** (5 years, 10 years). The longer
  the horizon, the more conservative the discipline; 50 years
  is the spec's stated goal. Shorter horizons would allow
  shorter deprecation windows; the engine's policy is the same
  regardless.

## Verification

- `cargo-semver-checks` runs on every PR (gate job in CI). Phase
  0 added; lives in `.github/workflows/ci.yml`.
- The backwards-compat integration test (Phase 10+) consumes the
  previous engine-api version and verifies a known consumer
  builds.
- The grep guard (PR 0 Commit G) rejects internal `engine_*`
  imports from bin/templates/starter-kits.
- The semver-checks baseline is the *latest* released
  `engine-api` version; PR-time comparison is against this
  baseline.
- ADR-050 documents the engine-api activation path: when the
  first non-trivial re-export lands, the engine-api version
  rolls from 0.0.x to 0.1.0 and the contract becomes
  operational.
