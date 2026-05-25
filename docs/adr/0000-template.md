# ADR-NNNN — <Short imperative title>

- Status: Proposed | Accepted | Superseded by ADR-MMMM | Deprecated
- Date: YYYY-MM-DD
- Phase: <0 / 1 / … / 5> — <PHASE NAME>
- Companion: <links to related ADRs by number, e.g. ADR-031, ADR-033>

<!--
Template for every new engine ADR. Copy this file to
`docs/adr/NNNN-<kebab-case-title>.md` and replace each placeholder
with the actual content for the decision being recorded.

ADRs are decision records, not feature specs. The goal is to leave
a future reader (yourself, six months from now; a new contributor,
five years from now; an auditor, 50 years from now) enough context
to understand *why* the decision was made — not just what it was.

Keep the prose specific. Cite spec sections, ADR numbers, file
paths, and crate names when they exist. Avoid hand-wavy language
("this is more flexible") in favour of concrete reasoning
("this lets PR N skip the migration step").

If the ADR is the planning record for code that will land later,
say so explicitly in the Status line ("Accepted (planning record;
implementation lands in Phase X PR Y)").
-->

## Context

<!--
What is the situation that forced this decision? What are the
constraints, the prior art, the failure modes of the obvious
approaches? What does the spec say (if anything)? What other ADRs
or files in the repo already make load-bearing decisions in this
area?

Aim for enough background that a reader who has not seen the
related spec sections can follow the rest of the ADR. Cite
sources (spec section numbers, ADR numbers, file paths).
-->

## Decision

<!--
What was decided. Be specific. If the decision has multiple parts,
number them. If the decision is "ship a contract / trait / type,"
include the trait signature here in a code block.

If the decision is a choice between named alternatives (e.g.
"BLAKE3 vs SHA-256"), name the chosen one and reference the
"Alternatives considered" section for the others.
-->

## Rationale

<!--
Why this decision and not the alternatives. The "Alternatives
considered" section lists what else was considered; this section
explains why *this one* won.

Concrete reasoning: cite measurements, prior art, spec
constraints, future-proofing arguments (with their horizon).
Avoid "we felt this was best" in favour of "the alternative
violates contract X" or "the alternative pays Y cost on Z path
that we cannot afford."
-->

## Consequences

<!--
What this decision implies for the rest of the engine. Includes:

- New crate dependencies.
- New CI gates or tooling.
- New file formats or contract surfaces.
- Constraints on future work (e.g. "any future system in this
  area must satisfy contract X").
- Concrete code-level implications (e.g. "every system that
  touches Y must declare Z").

This section is the load-bearing one for future readers: it tells
them what they can and cannot do in the wake of this ADR.
-->

## Risks and tradeoffs

<!--
What could go wrong, and how the decision mitigates or accepts
each risk. If a risk is accepted (no mitigation), say so plainly:
"acknowledged, no mitigation planned."

Common risk categories: performance, security, maintenance,
upstream dependency churn, build complexity, contributor learning
curve.
-->

## Alternatives considered

<!--
Each alternative considered seriously enough to be worth recording.
For each: what was it, why it was rejected. The goal is to save a
future reader the cost of re-deriving the comparison.

Format: bullet per alternative; one-paragraph or sub-bulleted
explanation; close with "Rejected" + brief reason.
-->

## Verification

<!--
How the decision is verified, end-to-end. Includes:

- Test files (cite paths: `crates/foo/tests/bar.rs`).
- CI jobs (cite workflow file + job name).
- Oracle gates (which oracle catches violations).
- Manual review steps (if any).
- Future-phase verification (cite the Phase X ADR or PR that
  will close this loop).

If no verification is possible today (the ADR is purely planning),
say so and name the phase that will make verification real.
-->

<!--
Optional addendum sections:

## Addendum (YYYY-MM-DD) — <reason>

Use when the ADR's decision is amended after a related PR or audit.
The original Decision/Rationale stays; the addendum captures the
new context, the new sub-decision, and the rationale for the
amendment. Pattern matched by ADR-033's "Addendum (2026-05-20) —
Engine Core v0.1.1: milestone closed" — informative for big-rock
follow-ups that ship within the same major thrust.
-->
