# ADR-020 — Telemetry consent is opt-in

- Status: Accepted
- Date: 2026-05-18 (expanded 2026-05-24 per audit §15 Phase-0 ADR sweep)
- Phase: 0 (pre-v1.0 — contract-exempt per risk R-03)
- Companion: ADR-010 (telemetry as first-class subsystem — this ADR
  is the consent layer), spec §X.8 (privacy), spec §XIX (privacy and
  data handling)

## Context

The engine emits telemetry — frame-pacing samples, GC pauses, asset
loads, crash reports, error counts. This data is invaluable for
diagnosing real-user issues; it is also subject to privacy law:

- **GDPR** (EU): explicit, informed, granular consent for any
  personally-identifying data leaving the device. Right of access,
  rectification, erasure.
- **CCPA** (California): opt-out for sale of personal information;
  consumer-rights surface.
- **COPPA** (US, under-13 users): explicit parental consent
  before any data collection.
- **PIPEDA, LGPD, PDPA, …** (Canada, Brazil, Singapore, …): per-
  jurisdiction variants of the same general consent principle.

The engine cannot ship "telemetry on by default" — the legal
exposure is significant, and the team's own privacy stance
(spec §XIX) rejects it on principle.

## Decision

Telemetry consent is **opt-in**, region-aware, and granular:

### 1. No telemetry leaves the device until consent is recorded.

- The engine ships with an empty consent profile.
- On first launch, the editor (Phase 10+) presents the consent
  UI (text, granular checkboxes, "decline all" prominent).
- Until consent is recorded, the local IPC channel (ADR-010)
  remains operational for in-process tooling, but no
  network-egress occurs.

### 2. Region-aware UI.

- The first-launch consent UI adapts to the user's locale.
  GDPR-jurisdiction users see GDPR-compliant text and the full
  rights surface (access/erasure links).
- COPPA-jurisdiction settings (US under-13) require parental
  email verification before any consent.
- The engine's locale detection (Phase 9+) drives this; pre-
  Phase-9 the consent UI is a placeholder.

### 3. Granular categories.

- `category::crash_reports` — opt-in for crash report upload.
- `category::performance_telemetry` — opt-in for frame-pacing
  and resource-usage data.
- `category::usage_analytics` — opt-in for editor feature usage
  counts (which menus are clicked, which tools used).
- `category::ai_provider_cloud` — explicit opt-in for cloud GM
  providers (ADR-017); a session that uses
  `Provider::CloudClaude` requires this consent.
- Each category is independently toggleable.

### 4. One-line delete-my-data endpoint.

- The engine ships a template (`templates/delete-my-data.tmpl`)
  that game shippers can plug into their backend. The
  endpoint accepts the user's identifier and returns a 204
  acknowledging deletion; the engine includes a CLI for the
  user to invoke locally
  (`engine privacy delete-my-data --user-id ...`).

### 5. Consent is auditable.

- The consent record is persisted to
  `~/.config/engine/consent.toml`.
- A consent change is itself a telemetry event (locally, with
  the consent gate not yet active) — so the audit trail is
  available even if the user later revokes consent.

## Rationale

Three reasons opt-in is the only acceptable posture:

1. **Legal exposure.** GDPR fines are 4% of global revenue;
   the engine's commercial deployments cannot afford a single
   misstep.
2. **The team's stated value.** Spec §XIX names privacy as a
   first-class design constraint, not a compliance afterthought.
3. **Reputation effect.** "Engine that respects user privacy"
   is a differentiator the team values; opting users in by
   default would damage the brand.

The granular categories let users consent to the parts they
find acceptable (e.g. "crash reports yes, usage analytics no")
without forcing an all-or-nothing decision.

The one-line delete-my-data template means game shippers can
support GDPR Article 17 (right to erasure) without each
having to design the endpoint from scratch.

## Consequences

- The engine's network-egress path is gated by the consent
  layer. Egress code that bypasses the gate is a CI failure
  (Phase 10+ when the egress code exists; pre-Phase-10 the
  gate is theoretical).
- The first-launch UI is the editor's job; pre-editor (pre-
  Phase-10) the engine has no UI to render the consent screen
  — and no network egress to gate. The contract is fully
  active from Phase 10.
- The consent record's persistence format (TOML at
  `~/.config/engine/consent.toml`) is part of the data-format
  contract (ADR-012's 50-year stability claim covers it).
- The "decline all" path is *fully supported* — the engine
  works without any telemetry upload. No feature is gated on
  consent.
- The cloud AI provider (ADR-017) gates on a separate consent
  category; declining AI cloud is fine, the local providers
  still work.

## Risks and tradeoffs

- **Opt-in fatigue.** Users may decline everything by default
  and the engine's diagnostic data flow goes dark. Acceptable:
  the team's privacy stance is the priority; the engine works
  without telemetry.
- **Diagnostic quality.** With low opt-in rates, the
  engine's understanding of real-user issues is reduced.
  Mitigation: in-house testing on the reference hardware
  captures most issues; community bug reports cover the rest.
- **Legal-text accuracy.** GDPR / CCPA / COPPA language is
  fiddly. Mitigation: the consent UI's text is reviewed by
  legal counsel before each engine release that ships to
  end-users.
- **Region detection accuracy.** A user travelling outside
  their home region may see the wrong consent UI on first
  launch. Mitigation: the UI is changeable post-first-launch;
  no irreversible decision is made.

## Alternatives considered

- **Opt-out by default.** Standard in pre-2018 engines; legally
  untenable for GDPR exposure; rejected on principle.
- **Telemetry-free engine.** Loses the diagnostic value of
  consented users. Rejected.
- **A single "telemetry on/off" toggle.** Coarse; doesn't meet
  GDPR's granularity requirement. Rejected.
- **Region-agnostic UI.** Loses the locale-aware compliance
  property; legally risky. Rejected.

## Verification

- Phase 10's editor implements the consent UI and the
  consent-record-persistence flow.
- The network-egress path's gate is verified by a unit test:
  with consent absent, an attempted egress is a typed error.
- The legal text is reviewed pre-release.
- The `delete-my-data` template is verified end-to-end by a
  reference-shipper integration test (Phase 10+).
- The consent file's schema is part of the data-format
  migration contract (ADR-012 / ADR-054); future schema
  changes require migration functions.
