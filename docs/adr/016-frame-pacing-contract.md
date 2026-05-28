# ADR-016 — Frame Pacing Contract

- Status: Accepted
- Date: 2026-05-18 (expanded 2026-05-24 per audit §15 Phase-0 ADR sweep)
- Companion: ADR-047 (frame pacing CI gate — the operationalisation),
  ADR-053 (Phase 5 PR slicing — Phase-5 PR 6 activates the gate),
  spec §IV.5

## Context

A game engine's user-perceptible quality is not "average FPS" — it
is *frame-to-frame consistency*. A title that averages 60 FPS but
periodically hitches at 30 FPS feels worse than a title that
sustains 50 FPS without hitches. The standard literature
(Carmack's "frame pacing" essays, Tim Lottes's analyses) converges
on the same headline metric: **p99 frame time and frame-time
standard deviation** are the headline pacing metrics, not mean
frame time.

The engine's spec §IV.5 sets the contract:

- The simulation tick (the ECS scheduler's frame) and the render
  frame are decoupled (interpolation between simulation ticks on
  the render thread).
- The render frame's p99 must fit inside the platform's frame
  budget (16.6 ms at 60 Hz, 11.1 ms at 90 Hz, 8.3 ms at 120 Hz).
- The frame-time standard deviation σ must fit inside a tight
  fraction of the frame budget — the spec calls for σ ≤ 1.04 ms
  on the Phase-5 milestone (a hit-rate equivalent to "no
  observable jitter at 60 Hz on the reference monitor").

## Decision

The Frame Pacing Contract has three numeric targets and one
metrics protocol:

### Numeric targets (Phase-5 milestone, RX 580 @ 1440p @ 60 Hz)

- **p99 frame time ≤ 18.3 ms** (10% headroom over the 16.6 ms
  budget; the gate is 18.3 ms because the spec acknowledges
  occasional vsync alignment slop).
- **σ ≤ 1.04 ms** (standard deviation of frame intervals over a
  60-second steady-state capture).
- **mean ≤ 16.6 ms** (the trivial precondition; if mean misses,
  p99 cannot meet target).

### Metrics protocol

- Frame-time samples are captured per frame on the render thread
  via `engine_platform::Instant::now()` (the engine's owned
  high-resolution timer wrapper, monotonic).
- Statistics are computed over a 60-second steady-state window
  (post-warmup; the first 5 seconds are excluded).
- The `bin/engine-bench-frame-pacing/` harness runs the
  measurement; output is owned-format JSON.
- ADR-047 wires this into CI as a required gate (the self-hosted
  GPU runner runs the harness on every PR touching the
  rendering stack).

### Operational contract

- A PR that regresses p99 frame time or σ is a CI failure.
- Per-game tuning: a game can declare its own pacing budget via
  `engine.toml` (Phase 6+; Phase 5 uses the engine's portfolio
  default).
- Regression *baseline* is the milestone-baseline measurement
  recorded in `docs/observatory/phase-5-milestone-baseline.md`.

## Rationale

p99 and σ are the metrics because mean FPS hides the
user-perceptible hitches. A 60 FPS mean with one 50 ms hitch
per second is not 60 FPS in any meaningful sense; it is "60
FPS that the user notices stuttering through."

The numeric targets come from the Phase-5 milestone constraint:
RX 580 at 1440p is the spec's reference. The 18.3 ms p99
threshold has 10% headroom over the 16.6 ms vsync interval
because OS-side jitter (compositor blits, GPU scheduling slop)
adds slop the engine cannot control.

The 60-second measurement window is the standard frame-pacing
analysis interval; shorter windows (e.g. 5 seconds) are too
noisy; longer windows (>5 minutes) waste CI time.

The metrics protocol is owned — JSON emitter, owned arg parser
(precedent: `tools/cache-observatory/` per ADR-030's family).
The bench is a small CLI; portability is a goal.

## Consequences

- Phase 5 PR 6 ships the gate (ADR-047). Pre-Phase-5 PRs do not
  have a frame-pacing gate (the renderer isn't real yet).
- The self-hosted GPU runner is the CI's measurement
  environment (per ADR-047 §Risks; cold-spare per ADR-047 §6).
- `docs/observatory/phase-5-milestone-baseline.md` is the
  baseline file; PR comparisons go against this baseline.
- A milestone slip (RX 580 cannot meet the gate) is a Phase-5
  closure blocker.
- The Phase-5 portfolio scene
  (`testbed/frame-pacing/scenes/v0.ron`) is the fixed test
  workload. A change to the workload is a separate PR (the
  gate's baseline regenerates).

## Risks and tradeoffs

- **Self-hosted runner reliability.** The runner must be
  available for every PR; outages block merges. Mitigation:
  ADR-047 §6 calls for a cold-spare and a runbook
  (`docs/runbooks/frame-pacing-runner.md`, PR 6).
- **Driver / kernel variance.** A driver update on the runner
  can cause a non-engine-attributable regression. Mitigation:
  the runner's driver version is pinned; updates roll through
  via a dedicated runner-maintenance PR.
- **σ measurement noise.** A 60-second window has finite
  sample size; statistical confidence in σ at the 1.04 ms
  threshold is bounded. Mitigation: gate tolerance includes
  a 1.1× factor; ADR-047 §4 documents the tolerance window.
- **Frame-pacing portfolio scope.** One scene cannot represent
  all workloads. Mitigation: post-Phase-5, additional scenes
  land per workload class (open-world streaming, indoor
  combat, etc.); the gate runs the full corpus.

## Alternatives considered

- **Headline metric is mean FPS.** Doesn't capture jitter; not
  the standard literature target. Rejected.
- **Headline metric is p99 only** (drop σ). σ captures jitter
  pattern that p99 misses (a regular 10 ms hitch every second
  is invisible to p99 in a 60 s window if 99% of frames clear
  the threshold). Keep σ.
- **Headline metric is the GPU's frame interval.** Useful;
  rendered frame may finish well before vsync; the user
  perceives presentation time. The engine measures the render
  thread's frame interval, which is close-enough to the
  user-perceived interval given vsync alignment. Acceptable.
- **No CI gate; pacing is a release-time check.** Loses the
  cheap-PR-time regression detection; the gate amortises over
  many PRs.

## Verification

- ADR-047 operationalises this contract: CI gate, runner,
  baseline, runbook.
- The Phase-5 PR 6 milestone validation is the first proof
  the contract holds end-to-end.
- The `bin/engine-bench-frame-pacing/` harness's output
  schema is verified by an in-PR unit test (validates JSON
  shape before the gate is active).
- The observatory baseline file
  (`docs/observatory/phase-5-milestone-baseline.md`) is
  committed; CI compares each PR's measurement to it.
- The contract's longevity: future phases inherit it; phase
  6's mesh-shader work-graph (Track B) will be measured the
  same way.
