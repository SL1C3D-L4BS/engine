# ADR-069 — Engine vs Spec phase reconciliation

- Status: Accepted
- Date: 2026-05-27
- Phase: 5.5 — Track A GPU binding closure (the rename this ADR
  formalises)
- Companion: ADR-053 (Phase 5 PR slicing — the plan whose milestone
  was declared closed prematurely), ADR-068 (Phase 6 PR slicing — the
  plan whose work is actually Phase 5 closure), ADR-051 (acknowledged
  deviations register — engineering deviations have a register; this
  ADR is the *process* deviation companion), ADR-074 (wgpu Vulkan
  activation — the first ADR landed under the corrected naming),
  ADR-070 (frame-pacing re-baseline — calibrates against the spec's
  actual milestone hardware), ADR-077 (3D Gaussian Splatting
  architecture — the work that will open under the *true* Phase 6
  naming once Track A closes)

## References

- `~/Resources/documentation/ENGINE_SPECIFICATION_v2.0.md` Part XXI
  ("Implementation Phases"), in particular lines 1627–1637 — Phase 5
  ("Rendering Foundation Track A"; milestone "deferred PBR running on
  RX 580 at 60 FPS @ 1440p. Software/GPU pixel parity") and Phase 6
  ("Neural Rendering & Gaussian Splatting"; milestone "3DGS scene >
  60 FPS. DLSS 4 / FSR 4 / XeSS 2 integrated, owned fallback").
- Spec Part XXIII ("Specification Contract") — "Every section that
  changes requires either an ADR in `docs/adr/` or a documented
  rationale in the git commit message."
- *Game Engine Black Book: Doom* (Sanglard, 2018) — the
  section-by-section "what shipped vs what was planned" engineering
  accountability discipline this ADR adopts.
- Brooks, *The Mythical Man-Month* (1975), Ch. 5 ("The Second-System
  Effect") — the diagnostic frame for the over-scoped "Phase 6"
  rename that absorbed unfinished Phase 5 work.

## Context

A pre-Track-A audit (the plan recorded at
`~/.claude/plans/radiant-enchanting-cocoa.md`) compared the engine's
phase state to the spec's. The audit surfaced three structural
deviations:

### 1. Spec Phase 5 milestone was declared closed without proof on the spec's named hardware

Spec line 1631 names the Phase 5 milestone as "deferred PBR running on
**RX 580** at 60 FPS @ 1440p." The `Engine Core v0.2` tag (commit
`11dc8a4`, 2026-05-27) landed with the renderer's CPU oracle and the
trait surface complete, but:

- No render-pass `record()` body called `begin_render_pass`.
- Every compute-pass `record()` body issued `dispatch_workgroups(1, 1,
  1)` as a placeholder (audit found this in seven passes).
- The workspace `Cargo.toml` `wgpu` dep had no backend feature
  enabled, so wgpu could not reach a real adapter at all.
- The frame-pacing milestone bench used a synthetic CPU workload
  (`bin/engine-bench-frame-pacing/src/main.rs` lines 24–29 are explicit:
  "GPU-backed numbers land when the self-hosted GPU runner
  stands up in PR 6").
- That self-hosted runner was never provisioned.

The "milestone met" claim was therefore architectural-aspiration, not
measured-fact. The honest reading: spec Phase 5 closes when the
milestone is *measured* on the spec's named hardware.

### 2. The "Phase 6" naming absorbed unfinished Phase 5 work

The engine opened "Phase 6 — Rendering Foundation Track A Part 2" via
ADR-068 (2026-05-27) with PRs 1–8. Inspecting the PR scope (mesh /
material owned formats, glTF importer subprocess, shader-artefact
ingest, GPU pass contracts, GPU pipeline binding, WGSL shader sources,
attachment-view plumbing, pixel-parity fixtures, frame-pacing gate
flip) reveals that **all of it is GPU-binding work** — the back half
of the spec's Phase 5 "wgpu PBR deferred · CSM · IBL · TAA · cluster
lights" deliverable (spec line 1628). Only one line of work (the
`OwnedOnnxTemporal` cascade reservation in PR 5) is genuinely spec
Phase 6 surface (the owned-upscaler fallback per spec line 1636).

This is Brooks's "second-system effect" applied to phase numbering:
the engine over-scoped Phase 5's close (declaring it done before it
was), then opened a Phase 6 that swelled to absorb the leftover work
under a name that promised future-phase scope (3DGS, neural rendering)
without delivering it.

### 3. Spec Phase 6 has zero code in tree

The audit confirmed: no `gaussian_splat.rs`, no `splat` module, no
neural-rendering compute shaders, no radix-sort pass, no composite
pass, no `KHR_gaussian_splatting` reader. Spec Phase 6's *real*
deliverable (the 3DGS renderer + the trained ONNX temporal upscaler +
the working vendor upscaler cascade) is unstarted.

## Decision

### 1. Phase numbering correction

The engine's current "Phase 6" work is renamed **Phase 5.5 — Track A
GPU binding**. The number "6" is reserved for the spec's true Phase 6
(3DGS + neural rendering + working vendor upscaler cascade) and opens
*after* Engine Core v0.3 ships.

| Engine state | Spec mapping | Tag | engine.toml `phase` |
|---|---|---|---|
| Pre-2026-05-27 PR 1 of "Phase 6" | Phase 5 closure work | v0.2 (premature) | `"5"` |
| 2026-05-27 PRs 1–7 of "Phase 6" landed | Phase 5 closure (continuing) | — | `"6"` (drift) |
| Now (this ADR) | Phase 5.5 — Track A GPU binding | — | `"5.5"` |
| All of Track A landed (per plan) | Spec Phase 5 milestone *measured* on RX 580 | v0.3 | `"5.5-closed"` |
| Track B opens (3DGS + neural rendering) | Spec Phase 6 (true) | — | `"6"` |
| Track B landed | Spec Phase 6 milestone met | v0.4 | `"6-closed"` |
| Phase 7 opens | Spec Phase 7 (Physics + 2D) | — | `"7"` |

### 2. Naming discipline going forward

- An engine "Phase N" name MUST match the spec's Phase N scope. If
  scope expands or contracts, a phase-rename ADR (amending this one)
  is required *before* PRs land under the new naming.
- A milestone declared "met" MUST be backed by a measurement on the
  spec's named hardware (or a documented deviation entry per ADR-051).
  Architectural-aspiration claims ("the code can do this in principle")
  do not qualify.
- A tag (v0.N) MUST correspond to a met milestone, not to an
  intermediate engineering close. v0.2 stays as the tag for the trait
  surface + CPU oracle close; v0.3 will tag the *measured* Phase 5
  milestone on real hardware.

### 3. Document touch-up

The corrective work that lands alongside this ADR:

- `engine.toml` `phase = "6"` → `phase = "5.5"` (then `"5.5-closed"`
  at v0.3, then `"6"` at Track B open).
- `README.md` Status section: the existing "Phase 6 (RENDERING
  FOUNDATION, Track A, Part 2)" paragraph is renamed to "Phase 5.5 —
  Track A GPU binding closure"; a new "Phase 6 (spec — Neural
  Rendering & Gaussian Splatting) — opens with Track B" paragraph
  follows.
- ADR-068 (Phase 6 PR slicing) gains a fifth close addendum naming
  this reconciliation and noting that its in-progress work is now
  formally Phase 5.5.
- The memory entries `phase-6-progress`, `phase-5-design-decisions`,
  `engine-monorepo-status` get closure addenda — not deletion —
  pointing at this ADR and the new track structure. History matters;
  the drift narrative is part of the record.

### 4. ADR-051 is unchanged

ADR-051 is the *engineering* deviations register (TOML breakpoints
vs spec RON, owned i18n parser vs ICU4X, owned binary IPC vs
MessagePack, ORT for owned ONNX upscaler). It is not the right home
for a *process* deviation (premature milestone close, scope drift in
phase naming). This ADR is the process-deviation companion to
ADR-051's engineering-deviation register.

## Rationale

- **Honest milestone discipline is a maintainability property.** A
  tag (v0.N) that doesn't correspond to a measured milestone produces
  false confidence in the work landed and obscures the actual
  technical state. Sanglard's *DOOM* book is the model: every
  section accounts for what shipped vs what was planned, and the
  divergence is part of the story.
- **Spec naming is the contract** (spec Part XXIII). Drifting from
  spec naming silently erodes the contract; documenting the drift in
  an ADR is the spec-prescribed mechanism for recording a change.
- **The work landed is not wasted.** Engine "Phase 6" PRs 1–7 + 7.5
  (mesh / material / glTF importer / shader binding / GPU pass
  contracts / WGSL shaders / pipeline binding / code-review follow-up)
  are real engineering progress. They are simply Phase 5 closure
  work, not Phase 6 opening work. Renaming them does not invalidate
  them; it makes the spec mapping correct.
- **The second-system effect diagnosis** (Brooks Ch. 5) is the
  textbook frame: a "next phase" that swelled to absorb leftover
  work is a known anti-pattern. The remedy is the corrective
  rename, not silent acceptance.

## Consequences

- One round of engine.toml + README edits + memory updates lands
  with this ADR.
- ADR-068's close addenda continue to be valid as engineering
  records of what shipped; the fifth addendum documents the rename.
- Future ADRs that file under the v0.3 closure work cite "Phase 5.5
  — Track A GPU binding" as the phase tag. ADR-074 (the first under
  the new naming) is the canonical example.
- `git log` shows the rename in the commit history; no rewriting of
  past commits.
- Memory entries gain closure addenda but stay in place for
  forensics.

## Risks and tradeoffs

- **Renaming an in-progress phase mid-stream is unusual.** Future
  readers may need a moment to map the rename — that moment is the
  reading of this ADR. The trade-off is favourable: a one-time
  rename cost vs. permanent phase-vs-spec confusion.
- **`engine.toml` `phase` field is now non-monotonic** (5.5 lands
  between 5 and 6 in name order but in time order it is after 6
  briefly existed). Document that the spec's phase numbering is the
  contract and the engine's `phase` field tracks the *spec* mapping,
  not its own monotonic counter.
- **The "Phase 6" mention in PR descriptions of commits b205450,
  d70b853, eec6aa4, 1faa877, 1dfd950, 7148f29, bb14ac4, d307c4b,
  f7bf287, 1501720, 7046d33, 9121775 is now historical.** Their PR
  descriptions remain valid as engineering records; reading them
  now requires knowing about this ADR.

## Alternatives considered

- **Keep the "Phase 6" name; file no ADR.** Continues the drift
  forever. Rejected — spec naming is the contract.
- **Renumber engine "Phase 6" to "Phase 5b" or "Phase 5c."**
  Closer to spec, but invents engine-specific lettering not present
  in the spec. "Phase 5.5" is closer to a decimal subdivision the
  reader can map directly to "between Phase 5 and Phase 6."
- **Roll back the v0.2 tag to "v0.2-trait-surface" + tag the real
  Phase 5 close as v0.3.** Preferred outcome and what this ADR
  formalises: v0.2 keeps its tag (no destructive history rewrite);
  v0.3 is the measured-milestone tag.
- **Open spec Phase 6 (3DGS) now in parallel with the Phase 5.5
  closure.** Tempting but premature: spec Phase 6 depends on the
  Phase 5 milestone being met (the deferred renderer that 3DGS
  composites against must work first). Sequential ordering per the
  spec is correct.
- **File this ADR as an amendment to ADR-068 instead of a new ADR.**
  ADR-068 is the engineering plan for what became Phase 5.5; this
  ADR is a *process* decision (how the engine maps to the spec). The
  separation is cleaner: ADR-068 is a closed plan with addenda;
  ADR-069 is a forward-looking discipline.

## Verification

- This ADR file is the verification artefact.
- `engine.toml` reads `phase = "5.5"` after the corrective edits
  land in the same commit / PR.
- `README.md` Status section reads "Phase 5.5 — Track A GPU binding
  closure" after the same commit.
- The memory entry `phase-6-progress` carries a closure addendum
  pointing at this ADR.
- ADR-068's fifth close addendum exists and points at this ADR.
- The reconciliation table in §1 is the canonical mapping; any
  future ADR or PR that names a "phase" cites this table.
