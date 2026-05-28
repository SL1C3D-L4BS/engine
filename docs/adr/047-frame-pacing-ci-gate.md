# ADR-047 — Frame Pacing CI gate

- Status: Accepted (Phase 5 design contract; gate activates in Phase
  5 PR 6)
- Date: 2026-05-24
- Phase: 5 — RENDERING FOUNDATION (Track A)
- Companion: ADR-016 (Frame Pacing Contract — this ADR specifies
  the mechanism the contract demands), ADR-039 (render graph),
  ADR-046 (rasterizer oracle)

## Context

Spec §IV.5 (Frame Pacing Contract) and ADR-016 declare:

- p99 frame time ≤ 1.1× target (e.g. 18.3 ms at 60 FPS target)
- σ ≤ target/16 (e.g. 1.04 ms at 60 FPS)
- "CI runs a 60-second standard-scenario test on every commit;
  regression past the budget fails the build."

The contract is real. The mechanism is unwritten:

- What is the "standard scenario"?
- Which CI runner? GPU hardware quality affects p99 dramatically.
- Is the metric p99 alone? σ alone? Both?
- Is it a strict regression threshold or a moving baseline?
- What is the appeal procedure when the gate fires on a real,
  intended quality bump?

ADR-016 is currently a 12-line stub. This ADR fills it in — and is
written as a *new* ADR (not a revision) per the spec's
immutable-ADR rule. ADR-016 is updated only to add a one-line
back-reference to this ADR.

## Decision

### 1. Standard scenario — `bench.frame_pacing.v0`

A deterministic scenario fixture lives in
`testbed/frame-pacing/scenes/v0.ron` and is mounted at PR-CI time. It
contains:

- 10 000 entities (the Phase 3 stress ECS), distributed across 50
  unique meshes.
- 64 lights (16 directional cascaded, 48 point/spot in the cluster).
- Day-night cycle camera path: 60-second deterministic camera
  motion path (camera position keyed by `BLAKE3(seed=0, frame)`),
  rotating across the scene to hit varying overdraw, shadow, and
  cluster-density.
- Default quality preset (RX-580 tier per spec Part XX.7).

The scene fixture is content-addressed (ADR-008) so it is
reproducible and any change to the fixture itself is reviewable.

### 2. Hardware envelope — self-hosted GPU runner

The gate runs on a self-hosted CI runner with an actual GPU. Initial
hardware: **an RDNA2-class GPU** (Mesa RADV, Vulkan 1.3,
RDNA 2). The RDNA2-class GPU is one tier above the RX 580 milestone
target and represents a stable, widely-available reference. It is
*not* the RX 580 (those are scarce and price-volatile for CI use);
the gate's targets are calibrated to the original CI runner and a separate
"milestone bench" on actual RX 580 hardware runs as a nightly job.

GitHub-hosted runners are not used for this gate — they have
neither dedicated GPUs nor latency stability.

CPU envelope: an 8c/16t Zen 3 proxy CPU (locked frequencies).
Memory: 32 GiB DDR4-3200 (2× 16 GiB, dual channel).
OS: Arch Linux on `linux-cachyos-bore` kernel per spec Part XVIII.4.

The runner is treated as production hardware: dedicated, no other
CI workload competes during a frame-pacing job. Job runs once per
push to default branch + once per PR commit on a render-touching path.

### 3. Metric — both p99 and σ, hard thresholds

The 60-second scenario yields 3 600 frames at the 60 FPS target. Per-
frame `frame_time_ns` is captured by the existing telemetry layer
(SPAN "frame.total" rolled up).

```
PASS  iff   p99(frame_time_ms) ≤ 18.3   AND   stddev(frame_time_ms) ≤ 1.04
```

Both thresholds must hold. p99 alone is gameable (engine could
intentionally tank σ to hit p99); σ alone misses single-frame stalls.
The two together match ADR-016 (Frame Pacing Contract).

### 4. Regression band — strict vs. moving

**Strict, with explicit appeal.** The thresholds (18.3 ms p99, 1.04
ms σ) are the spec's published contract; the gate fails if either is
exceeded. No moving baseline.

The appeal procedure (5) is the formal way to land an intentional
quality bump that pushes against the threshold.

### 5. Appeal procedure — three options for a deliberate change

When a PR legitimately raises p99 or σ (a new high-quality post-FX
pass, a more expensive material model, etc.), the PR author chooses:

**Option A — Reduce default quality preset.** Lower the default
post-FX settings (e.g. drop bloom kernel from 5 mips to 4) so the
RDNA2-class GPU preset still hits 18.3 / 1.04. The new high-quality
work is gated behind a quality preset. The fixture scene runs at the
default preset, so the gate passes.

**Option B — Approve a calibrated threshold bump.** If the
expensive feature is essential and cannot be preset-gated, a separate
ADR is required to revise the spec-stated thresholds. ADR-047A,
047B etc. would each name the new threshold, the rationale, and the
rollback plan. The CI gate file (`tools/frame-pacing/budgets.toml`)
is updated as part of that ADR's PR.

**Option C — Re-baseline on different hardware.** If the original CI runner
is genuinely too old, the runner hardware moves up a tier. Same
process as Option B (an ADR records the move; budgets re-calibrate).

In all three cases, an ADR records the decision. Silent moving
baselines are forbidden — they hide regression debt.

### 6. CI integration

A new job in `.github/workflows/ci.yml`:

```yaml
frame_pacing:
  runs-on: self-hosted-gpu
  if: ${{ contains(github.event.head_commit.modified, 'crates/engine-render')
       || contains(github.event.head_commit.modified, 'crates/engine-gpu')
       || contains(github.event.head_commit.modified, 'engine.toml') }}
  steps:
    - uses: actions/checkout@v4
    - name: Run frame-pacing scenario
      run: cargo run --release -p engine-bench-frame-pacing
        -- --scene testbed/frame-pacing/scenes/v0.ron
           --duration 60s
           --output /tmp/frame-pacing.json
    - name: Evaluate gate
      run: cargo run --release -p engine-bench-frame-pacing
        -- --gate /tmp/frame-pacing.json
           --budgets tools/frame-pacing/budgets.toml
```

The job runs *only* on PRs that touch render-relevant paths
(branch-aware filter). Cold-cache full runs land on main pushes; PR
runs use the sccache cache.

The job is required to pass for the gate to be enforceable; making
it required happens in Phase 5 PR 6 (the closure PR for Phase 5).

### 7. Pre-Phase-5 status

Until Phase 5 PR 6 lands, the job exists in the workflow as
`continue-on-error: true` (informational). Any regression surfaces
in PR comments without blocking merge. The transition to required-
status happens with the same PR that ships the RX-580 milestone
bench.

## Consequences

- The Frame Pacing Contract becomes operational, not aspirational.
- One new bench binary: `bin/engine-bench-frame-pacing/` (lands with
  Phase 5 PR 6). Owned arg parser, owned report format (JSON; same
  pattern as existing observatory baselines). Per the owned-vs-
  vendored discipline, no clap / serde_json — owned.
- Capacity planning: one self-hosted GPU runner. Operational cost
  ~$50-100/mo (same line item as the rasterizer-oracle runner —
  the same physical machine can host both jobs).
- The appeal procedure makes threshold changes visible and reviewable
  via ADR. No silent drift.

## Risks and tradeoffs

- **Self-hosted hardware is a single point of failure.** Mitigated
  by a cold-spare on a different physical host (spec §0.3 R-05);
  the cold-spare can take over with a few hours of runbook work.
- **Self-hosted runners require maintenance** (OS updates, sccache
  cleanup, GPU driver rolls). Documented in
  `docs/runbooks/frame-pacing-runner.md` (lands with Phase 5 PR 6;
  out of this ADR's scope).
- **The "branch-aware" job filter could be gamed** — a PR that
  changes a render-relevant file but adds the change in a way that
  doesn't trigger the filter. Mitigated by main-branch always
  running the gate.
- **Per-commit GPU-hardware runs cost compute.** Acceptable: the
  cost of a regression that escapes is higher.
- **σ ≤ 1.04 ms is tight.** Idle stalls from the kernel scheduler,
  page-cache hits, or GPU driver internals can spike σ on otherwise-
  healthy runs. The cachyos-bore + sched-ext stack from spec
  Part XVIII is the mitigation; the runner is pinned to that setup.

## Alternatives considered

- **GitHub-hosted CI runner with a software rasterizer.** Rejected:
  no GPU latency profile.
- **Cloud GPU runner (AWS g4ad, GCP T4).** Rejected: variable
  latency from neighbour noise; doesn't match the spec's bare-
  metal stance.
- **Moving baseline (last week's p99 + 5% headroom).** Rejected:
  hides creeping regressions. The spec-published targets are the
  baseline.
- **Run on a Steam Deck (handheld) as the reference.** Phase 11+
  candidate; mobile/console isn't a Phase 5 target.

## Verification

- Phase 5 PR 6 adds the bench binary, the workflow job, and the
  budgets file. The Phase-5 closure criterion is "the job runs
  green on the milestone PR."
- Telemetry source: the existing `SPAN("frame.total",
  Subsystem::Render)` span the renderer emits per frame. Roll-up
  computed by the bench binary.
- The ADR-016 stub gets a one-line update referencing this ADR
  (no behavioural change; the contract was already accepted).

## Addendum (2026-05-27) — superseded in part by ADR-070

ADR-070 (Phase 5.5 C.2 — frame-pacing re-baseline on the user's
RX 580 + local-only gate) supersedes the operational sections of
this ADR:

- §2 (hardware envelope) — the self-hosted GPU runner was
  never provisioned; ADR-070 §1 re-baselines on the developer's
  the developer's Skylake CPU + RX 580 (the spec's named Recommended-tier hardware).
- §3 (thresholds) — the spec values stand as the future-tier
  contract; ADR-070 §2 documents the measured baseline + the
  McKenney σ-floor analysis that frames the achievable envelope on
  Skylake 4c/8t.
- §6 (CI integration) — ADR-070 §3 removes the `frame_pacing` job
  from `.github/workflows/ci.yml` and replaces it with a local
  `just frame-pacing` recipe.
- §7 (informational vs required) — resolved: the gate is local; CI
  enforcement is not in play.

§1 (standard scenario), §4 (strict regression band as discipline),
and §5 (appeal procedure) stand. §5's appeal procedure is the
mechanism by which future re-baselines amend ADR-070.

This ADR is preserved as the engineering record of the prior plan.
