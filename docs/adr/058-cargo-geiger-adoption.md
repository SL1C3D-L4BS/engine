# ADR-058 — cargo-geiger adoption

- Status: Accepted
- Date: 2026-05-24
- Phase: 0 (foundation; CI step lands with Phase-0 catchup PR)
- Companion: ADR-025 (audited crypto crates — geiger enumerates
  their unsafe surface), ADR-001 (Rust as implementation language
  — geiger is the visible-unsafe mechanism), spec §XIX.2
  (security)

## Context

The engine is overwhelmingly safe Rust. The few `unsafe` blocks
that exist are load-bearing — the scheduler's `&mut World`
reborrow (ADR-033), the archetype storage's typed-erasure
(ADR-031), the fiber job system's `naked_asm!` (ADR-032), the
mmap loader's munmap-on-drop (ADR-029). Each is justified by an
ADR + an oracle.

What is missing: a *visible* enumeration of the unsafe code in
the workspace, including upstream dependencies. Spec §XIX.2
names `cargo-geiger` as the tool for this. The audit's §13 /
§16 noted the gap.

`cargo-geiger` is the Rust ecosystem's standard tool for
enumerating `unsafe` blocks: it walks the dependency tree and
counts unsafe items per crate (functions, traits, methods,
impls, expressions).

## Decision

`cargo-geiger` is the engine's unsafe-enumeration tool. CI
adoption:

### 1. Baseline file

`docs/observatory/cargo-geiger-baseline.md` is the committed
baseline. It contains:

- The workspace-wide cargo-geiger output, captured at audit
  close (2026-05-24).
- Per-crate row: unsafe-function count, unsafe-method count,
  unsafe-impl count, unsafe-expression count, total LOC of
  unsafe.
- A short explanation per crate that *has* unsafe (e.g.
  "engine-platform: 14 unsafe blocks in fiber/{x86_64,
  aarch64}.rs — see ADR-032").

### 2. CI step

The `.github/workflows/ci.yml` gate job runs:

```yaml
- name: cargo-geiger unsafe enumeration (ADR-058)
  run: |
    cargo geiger --workspace --output-format Json > geiger.json
    # Compare against the committed baseline; any drift is a PR-time
    # discussion.
    cargo run -p engine-geiger-check -- \
      --baseline docs/observatory/cargo-geiger-baseline.md \
      --report geiger.json
```

`engine-geiger-check` is a small workspace binary (~150 LOC)
that parses the JSON report and the markdown baseline, then
compares totals per crate. A new unsafe item is a soft warning
(the baseline must be updated in the same PR); a new unsafe
crate is a CI failure (review must add a baseline section).

### 3. Update discipline

New unsafe code lands by updating the baseline in the same PR.
The baseline diff is the visible review surface:

```diff
- engine-platform | unsafe_fns: 6 | unsafe_blocks: 12 | naked_asm: 2
+ engine-platform | unsafe_fns: 7 | unsafe_blocks: 14 | naked_asm: 2
+   2026-06-12: added unsafe { mmap_fixed } for ADR-070's
+   shared-memory ring buffer.
```

### 4. Scope

The baseline tracks **workspace crates** (everything under
`crates/`, `bin/`, `tools/`, `testbed/`). Upstream
dependencies' unsafe is captured for visibility (the
`cargo-geiger --workspace` report includes them) but is not
gate-controlled — the engine cannot modify upstream code, only
its dependency set (which is gated by `cargo-deny`'s allowlist
per ADR-025).

## Rationale

Two properties make `cargo-geiger` the right choice:

1. **Mature.** The tool has been the de-facto standard since
   2019; it works on every workspace shape and every dep
   topology. The audit's stance is to use the standard tool,
   not own one.
2. **Diffable.** The output is structured JSON that compares
   cleanly against a baseline. The audit's §16 reviewability
   property holds: a PR that touches unsafe is visible in
   the baseline diff.

The owned check (`engine-geiger-check`) is a thin tool — it
parses the JSON, compares against the baseline, formats the
diff. Phase-0 catchup PR ships it. Estimated ~150 LOC.

The decision to gate only on *new unsafe crates* (not *new
unsafe items*) follows the same risk model the audit uses:
adding an unsafe block to an existing carefully-bounded crate
is much lower risk than introducing unsafe into a previously
safe crate. The latter case warrants a deliberate review;
the former is routine.

## Consequences

- `cargo-geiger` is a build-time CI dependency. Install cost
  is small (a single binary, cached by the gate job).
- `engine-geiger-check` is a new workspace tool; ships with
  Phase-0 catchup. Lives under `tools/engine-geiger-check/`.
- `docs/observatory/cargo-geiger-baseline.md` is the
  authoritative baseline; every PR that adds unsafe updates
  it.
- Per-crate unsafe is *visible*, not *gated*. New unsafe
  inside engine-platform (which already has unsafe for
  fibers) is allowed; new unsafe in engine-script (which
  has none today) is a deliberate event.
- Future security audits inherit this baseline; an external
  reviewer can read the file and see the engine's unsafe
  surface in one document.

## Risks and tradeoffs

- **False positives.** `cargo-geiger` occasionally double-
  counts unsafe in macros. Mitigation: the baseline's "total
  LOC of unsafe" column is the authoritative human-readable
  metric.
- **Upstream churn.** A dependency update can change its
  unsafe surface; geiger reports it. Mitigation: dep updates
  go through `cargo-deny`'s allowlist (ADR-025) and the
  baseline update is part of the dep-update PR.
- **Maintenance cost.** Updating the baseline on every
  unsafe-touching PR is a small overhead. Mitigation: the
  diff is small and informative; the discipline pays for
  itself in audit clarity.
- **`cargo-geiger`'s own maintenance.** Mitigation: if the
  upstream tool stalls, fork or replace; the baseline format
  is the contract, not the specific tool.

## Alternatives considered

- **`cargo-deny`'s safety-related advisories.** Useful for
  CVEs; doesn't enumerate unsafe code. Complement, not
  replacement.
- **`miri`.** Detects UB at runtime; orthogonal to
  enumeration. The engine runs miri on selected tests where
  unsafe lives. Both tools coexist.
- **`cargo-careful`.** Stricter Rust ABI checking;
  complementary to geiger. Possible Phase 10+ adoption.
- **Hand-written enumeration.** Possible but quickly drifts
  out of date. Rejected.
- **No tool; rely on code review.** Misses the visibility
  property that the baseline file gives. Rejected.

## Verification

- `cargo geiger --workspace --output-format Json` runs in CI.
- The baseline file (`docs/observatory/cargo-geiger-baseline.md`)
  reflects the current state of the workspace.
- `engine-geiger-check` returns 0 when the report matches the
  baseline; non-zero when there's drift.
- Manual review on every PR touching the baseline confirms
  the discipline (the audit's R-04 reviewability property).
- The baseline is regenerated on engine-version bump; ADR-052's
  reproducibility cadence captures the dep-tree state.
