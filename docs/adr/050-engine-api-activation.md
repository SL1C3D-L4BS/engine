# ADR-050 — `engine-api` activation strategy and
`cargo-semver-checks` adoption

- Status: Accepted (CI step lands as part of audit remediation;
  activation date is bound to Phase 10)
- Date: 2026-05-24
- Phase: 5-prep
- Companion: ADR-012 (50-year API stability contract — this ADR is
  the operational realization), ADR-049 (engine-gpu wrapper —
  comparable owned-boundary discipline)

## Context

ADR-012 commits the engine to a 50-year API stability contract:
`engine-api` is the only crate game code imports, breaking changes
require major version bumps, `cargo-semver-checks` enforces the
contract in CI. The audit found:

- `crates/engine-api/src/lib.rs` is a 3-line doc-comment stub.
- `crates/engine-api/Cargo.toml` declares no dependencies.
- `.github/workflows/ci.yml` does not run `cargo-semver-checks`.
- The spec's risk register R-03 explicitly carves an exception for
  v0.x ("v0.x releases (pre-Phase 10) are explicitly contract-
  exempt; the contract activates at v1.0").

So the contract is *correctly* in dormant mode today. The audit's
recommendation is **not** to activate the contract prematurely. The
recommendation is to **wire the enforcement mechanism in now**, while
the surface is empty (zero risk of false positives), so the gate is
proven in-place when the contract goes live.

This is a small operational ADR, not a re-debate of the contract.

## Decision

### 1. Add `cargo-semver-checks` as a CI step (no-op today)

A new step in `.github/workflows/ci.yml`'s `gate` job:

```yaml
- name: Install cargo-semver-checks
  run: cargo install cargo-semver-checks --locked

- name: API stability check (engine-api, ADR-012 + ADR-050)
  run: |
    # Today: engine-api is a 3-line stub; the check is a no-op and exits 0.
    # The wiring is here so that the gate is proven in-place before the
    # contract activates at v1.0 (per spec risk R-03).
    cargo semver-checks check-release -p engine-api || exit_code=$?
    # cargo-semver-checks exits 0 when no public items exist; verify.
    if [ "${exit_code:-0}" -ne 0 ]; then
      echo "::error::engine-api API stability check failed — see ADR-012 / ADR-050"
      exit 1
    fi
```

The first time a public item lands in `engine-api` (likely Phase 10
when the editor + game-runtime API surface settles), this step
becomes active without modification. Until then it succeeds trivially.

### 2. Pin the v0.x → v1.0 activation to Phase 10

Per spec §XXI line 1659:

> --- v1.0 shippable here: the full vision. ---  (Phase 10)

This ADR pins the activation as a contract: when the engine ships
v1.0, the `engine-api` façade has frozen and `cargo-semver-checks`
becomes a required status check.

Until then, every release is v0.x. Public items added to `engine-api`
before v1.0 are subject to change — the package version's pre-1.0
status is itself the warning.

### 3. Future contract activation steps (Phase 10)

When the team approaches v1.0, the following land together (out of
this ADR's scope, recorded as the activation runbook):

- Move from `version = "0.x.y"` to `version = "1.0.0"` across the
  workspace (per-crate `workspace.package.version` already
  centralises).
- Add `tests/semver/` content — code samples that represent the
  promised v1.0 API. CI compiles them and runs them.
- Tighten the `cargo-semver-checks` step from "would-fail-on-
  breakage" to "must-succeed" (the current wiring already does
  this; the wording just acknowledges activation).
- Add the deprecation-policy enforcement: `#[deprecated]` annotations
  must include `since = "X.Y.0"` and a `note`, enforced by a
  clippy-extra lint or a custom CI grep.

The activation PR is a single coordinated change; the dormant wiring
this ADR installs is the prerequisite.

### 4. Why now, not at activation time

Three reasons:

- **Trust by drilling.** A CI step that has never run is more likely
  to fail at the critical moment than one that has run 1 000 times
  as a no-op.
- **Test the tooling.** `cargo-semver-checks` itself has versions;
  wiring it now surfaces installation friction (compile time,
  dependency conflicts) outside the activation window.
- **Cheap insurance.** The step adds ~15-30 s to the gate job; cost
  trivial.

## Consequences

- One new CI step. Workflow file gains ~10 lines.
- `cargo-semver-checks` becomes a workspace-level dev tool; documented
  in the `justfile` as `just semver` for local invocation.
- Phase 10 inherits a working gate. The activation PR is small.

## Risks and tradeoffs

- **`cargo-semver-checks` install time** adds to CI cold start. The
  `cargo install --locked` is cached the same way `cargo-nextest`
  and `cargo-deny` are.
- **False positives on a stub crate** — none possible; the crate has
  no exports. If `cargo-semver-checks` ever errors on an empty
  crate, the CI job emits the error and a follow-up ADR records the
  hand-off.
- **v0.x exemption could be abused** — a team could keep `engine-api`
  growing through v0.99 and never activate. Mitigated by ADR-012's
  spec promise and the Phase 10 milestone gate; the activation is
  on the release-team's checklist for v1.0.

## Alternatives considered

- **Wire it only when the first public item lands.** The activation-
  time-discovery risk is real; rejected.
- **Activate the contract now, before Phase 10.** Premature; would
  require freezing surfaces that aren't designed yet (render,
  physics, audio, ai). Rejected per spec R-03.
- **Use a different semver-check tool** (e.g.
  `cargo-public-api`). `cargo-semver-checks` is the spec-named tool
  (ADR-012); no reason to swap.

## Verification

- Lands as part of the audit-remediation CI extensions packet
  (task #17). PR ships:
  - One new step in `.github/workflows/ci.yml`.
  - One new `justfile` target (`just semver`).
  - First green CI run on the PR proves the wiring works on an
    empty `engine-api`.
- Re-verification at Phase 10 activation — separate runbook.

## Addendum (2026-05-24) — engine-api boundary grep guard

The Phase-0 catchup PR (PR 0 Commit G) adds a second mechanism
alongside cargo-semver-checks: a CI grep guard rejecting any
import of internal `engine_*` crates from `bin/`, `templates/`,
or `starter-kits/`.

While `engine-api` is empty (pre-Phase-10), the grep guard is
the *only* mechanism preventing `bin/` from accidentally
importing internal crates and turning them into de-facto
contract surface. The guard maintains an inline allowlist of
internal crates that today's CLIs already consume directly
(`engine_script`, `engine_asset`, `engine_core`,
`engine_telemetry`, `engine_platform` — used by Phase-0–4
CLIs and benches).

Future state (post-Phase-10): the guard tightens to "only
engine_api may be imported from bin/." At that point the
allowlist collapses to a single name. The transition is part
of the Phase-10 activation runbook.

The CI step lives in `.github/workflows/ci.yml`'s gate job,
named "Guard against internal engine_* imports from bin /
templates / starter-kits (ADR-050)."
