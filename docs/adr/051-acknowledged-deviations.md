# ADR-051 — Acknowledged deviations register

- Status: Accepted (living document — entries are added by ADR
  amendments)
- Date: 2026-05-24
- Phase: 0–4 retrospective
- Companion: every prior ADR whose decision is *deviated from* in
  the current code base

## Context

The audit (§1, §15) found three deliberate deviations from the spec
that have shipped to main:

1. `engine-script` writes breakpoints to a TOML file
   (`.engine/debug/breakpoints.toml`); spec §XII calls for RON.
2. `engine-i18n` ships an owned Fluent-subset parser + owned CLDR
   plural-rules + owned number formatting; the Phase-0 build plan
   named `fluent-bundle` + `icu` (ICU4X) + `unic-langid` as the
   third-party stack.
3. `engine-telemetry` ships an owned compact binary encoding for its
   IPC bodies; the Phase-0 build plan named MessagePack (`rmp-serde`).

Each deviation has a sensible rationale; each has a memory entry
(`[[foundation-layer-deviations]]`); none have been formalised in the
repository's ADR set. The audit's stance: **deviations should live in
the repo, not only in agent memory.** This ADR is the formal register.

It is a *living* document — new entries land via ADR amendments
(this same file gains new rows), not via new ADR numbers. Every
existing entry stays forever; sunset of a deviation (e.g. switching
engine-i18n to ICU4X) becomes a new entry that supersedes the prior.

## Decision

### 1. Deviation: TOML for breakpoint persistence

- **Spec:** §XII (Script Debugger) names RON as the on-disk format
  for `.engine/debug/breakpoints.ron`.
- **As shipped:** Phase 4 PR 3 ships `.engine/debug/breakpoints.toml`
  written by an owned ~80-line TOML writer/reader in
  `crates/engine-script/src/breakpoints_toml.rs`.
- **Why:** the repo had no RON parser at the time and the platform
  manifest layer already used TOML; adding a RON dependency for one
  file was a disproportionate side quest.
- **Why it's safe:** TOML is a stricter superset for the breakpoint
  schema's needs (key-value with simple types). Both formats are
  human-readable text; both round-trip cleanly.
- **Gate condition under which to revisit:** if engine-script ever
  adopts RON for another purpose (e.g. scene authoring lands and
  uses RON), the breakpoint format should consolidate. As of 2026-
  05-24, no other RON use exists in the engine source.
- **Acknowledged:** 2026-05-24 (this ADR). Implementation since:
  Phase 4 PR 3, 2026-05-20.

### 2. Deviation: owned Fluent subset + CLDR (engine-i18n)

- **Spec:** Phase 0 foundation build plan named `fluent-bundle` +
  `icu` (ICU4X) + `unic-langid` as the i18n stack for the editor's
  internationalisation layer (spec §II.6).
- **As shipped:** `crates/engine-i18n/` ships owned code: a Fluent-
  subset parser (handles `{$var}`, `{ DATETIME($d) }`, `{ NUMBER($n)
  style: "currency" }`; does not handle terms, message references,
  attributes); owned CLDR plural rules; owned number formatting.
- **Why:** ICU4X is a substantial dependency (compile time, binary
  size, transitive deps); the Phase-0 audit principle (spec R-02 —
  every owned subsystem ships with an oracle) made a vendored ICU4X
  hard to verify in the foundation layer; the engine's near-term
  i18n surface (the editor) doesn't yet need the full Fluent
  feature set or full ICU4X CLDR data.
- **Why it's safe:** the owned implementation passes round-trip tests
  against a curated fixture set; locale support is currently English
  + Japanese (test locales) and adding new locales is a contained
  data-only PR.
- **Gate condition under which to revisit:** when the editor's i18n
  surface grows to need Fluent terms, message refs, or attributes —
  or when the engine adds locale support beyond a handful of test
  locales — the engineering cost of expanding the owned parser
  exceeds the integration cost of ICU4X. Estimated trigger: Phase
  10 (editor enters production-quality localisation work).
- **Acknowledged:** 2026-05-24 (this ADR). Implementation since:
  Phase 0 foundation build.

### 3. Deviation: owned binary IPC (engine-telemetry)

- **Spec:** §X.5 names MessagePack as the IPC body encoding for
  telemetry IPC, accessed via `rmp-serde`.
- **As shipped:** `crates/engine-telemetry/` ships an owned compact
  binary encoding for the IPC frame bodies. No serde dependency.
- **Why:** the IPC frame header (`[frame_len:u32 | msg_type:u8 |
  flags:u8 | seq:u16]`) per §X.5 is already a fixed binary format;
  the body is small (typically a SPAN with two strings and four
  u64 fields); the size win of MessagePack vs. owned little-endian
  encoding is ~10-20% on the body, irrelevant on the IPC throughput
  budget; adding serde to the foundation layer would make
  engine-telemetry's dependency tree large and harder to oracle-
  verify. (The same reasoning ADR-036 applied to debug protocol.)
- **Why it's safe:** the owned encoder is small (~150 LOC) and has a
  round-trip oracle. Every IPC consumer (`engine-tui`,
  `engine-postmortem`, etc., Phase 10) will use the engine's
  decoder, not parse the bytes by hand.
- **Gate condition under which to revisit:** if a tool *outside* the
  engine repo needs to consume the IPC (e.g. an external metrics
  ingester at Hetzner's Prometheus instance), MessagePack's wider
  language support would make the engineering trade-off worthwhile.
  Until then, owned is right.
- **Acknowledged:** 2026-05-24 (this ADR). Implementation since:
  Phase 0 foundation build.

### 4. Deviation: ONNX Runtime as owned-upscaler backend

- **Spec:** §IV.4.A spec line 1634 names an owned ONNX temporal
  upscaler as the universal-coverage fallback. Spec R-02 prefers
  owned subsystems but is silent on the inference runtime.
- **As shipped:** ADR-067 §2 specifies the `OwnedOnnxTemporal`
  provider; the runtime binding consumes the `ort` crate (Rust
  wrapper around ONNX Runtime). The crate is gated behind the
  `ort-runtime` cargo feature in `crates/engine-upscale-vendor/`
  (Phase 6 PR 5.5 scaffold). The bundled model
  (`crates/engine-render/assets/onnx/temporal_upscaler_v1.onnx`)
  is content-addressed via BLAKE3.
- **Why:** training, exporting, and running a competitive temporal
  upscaler model on the engine's target hardware is a multi-year
  project. ONNX Runtime is the vendor-neutral standard with the
  broadest hardware-backend support (CUDA, ROCm, DirectML, CoreML,
  CPU). Owning the *model* + the *integration* + the *trait
  surface* while consuming the *runtime* matches ADR-025's "engine
  owns the use of the primitive, not the primitive itself" stance
  applied to crypto. The same precedent applies here.
- **Why it's safe:** the model is content-addressed and BLAKE3-
  verified at load time. The runtime version is pinned in
  `engine-upscale-vendor`'s Cargo.toml. Per-frame inference operates
  on GPU tensors (no untrusted-parse surface in the per-frame path).
  The `ort` crate itself is MIT-licensed Rust; the underlying ONNX
  Runtime is Apache-2.0.
- **Gate condition under which to revisit:** when a pure-WGSL
  inference path achieves competitive quality + perf without the
  ORT dependency (estimated trigger: 2030+ as wgpu's compute
  feature surface matures with INT8 matmul + f16 cross-backend).
- **Acknowledged:** 2026-05-27 (this addendum). Implementation
  status: **active** since Phase 5.5 A.4 (2026-05-28). The
  `OwnedOnnxTemporal` provider in `engine-render::upscale` returns
  `supports() = true` unconditionally and emits the cascade-selected
  token; the runtime falls back to
  `engine_raster::upscale::bilinear_upscale` when the `ort-runtime`
  cargo feature is off (default) and to a real `ort::Session` against
  the bundled model when on. The model artifact (`temporal_upscaler_v1.onnx`,
  Git-LFS tracked) is content separate from this code change;
  the runtime + cascade are content-agnostic and degrade gracefully.

### 5. Format for adding new entries

Future deviations land here via ADR amendments to this file:

```
### N. Deviation: <name>

- Spec: <citation>
- As shipped: <where it lives, what it does>
- Why: <rationale>
- Why it's safe: <verification / oracle>
- Gate condition under which to revisit: <trigger>
- Acknowledged: <date> (ADR-051 amendment N)
- Implementation since: <commit / phase>
```

A deviation that is *revisited and resolved* (e.g. engine-i18n
adopts ICU4X) gets a new entry under a new number describing the
resolution; the original entry stays for history.

## Consequences

- The repo now has a single canonical location for "intentional
  deviations from spec" — no more agent-memory-only knowledge.
- New deviations require a PR that amends this ADR, which forces a
  reviewable discussion before the deviation lands.
- The audit (§1.1, §15) can mark "acknowledged-deviation" entries
  with confidence that they're real-acknowledged (a PR-reviewed
  decision), not silently tolerated.

## Risks and tradeoffs

- **Living ADRs are unusual** — most ADRs are immutable. Pattern:
  ADR-016 was extended into ADR-047 rather than edited. The
  *register* pattern is the exception, justified because the
  *content* is a catalogue, not a decision.
- **A deviation can be added without sufficient review** if the
  amending PR is rubber-stamped. Mitigation: this ADR's review
  rubric (the "Why it's safe" + "Gate condition" fields) makes a
  shallow PR visibly shallow.
- **The agent-memory entry** (`[[foundation-layer-deviations]]`)
  becomes redundant once this ADR is in place. The memory entry
  should be updated to *point at this ADR* rather than recreate
  the content.

## Alternatives considered

- **Per-deviation separate ADRs.** Cleaner ADR numbering; loses the
  single-look-up benefit of a register. Rejected: deviations are a
  catalogue.
- **A markdown doc outside docs/adr/.** Loses the ADR review
  discipline. Rejected.
- **Spec amendments.** The spec is intentionally aspirational; the
  engine's deviations are intentionally pragmatic. Spec amendments
  would erode the "spec is the contract" property. Rejected.

## Verification

- This ADR file is the verification artefact.
- The agent-memory entry (`[[foundation-layer-deviations]]`) gets
  updated to point at this ADR (post-audit memory pass).
- A periodic spec-vs-shipped audit (the next time something like
  this audit runs) re-reads this register and confirms the listed
  deviations are still the only ones — or files new amendment PRs
  for new findings.
