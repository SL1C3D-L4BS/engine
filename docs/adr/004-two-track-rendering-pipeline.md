# ADR-004 — Two-track rendering pipeline

- Status: Accepted
- Date: 2026-05-18 (expanded 2026-05-24 per audit §15 Phase-0 ADR sweep)
- Phase: 0 (pre-v1.0 — contract-exempt per risk R-03)
- Companion: ADR-039 (render graph abstraction — the implementing
  contract for both tracks), ADR-046 (rasterizer oracle — the
  verification mechanism that lets Track B's parity claim be
  measured), ADR-053 (Phase 5 PR slicing — Track A delivery
  schedule)

## Context

The state of GPU rendering in 2026 is bifurcated. Two architectural
options coexist:

- **Track A — classic deferred PBR.** Forward-or-deferred geometry
  pass, clustered/forward+ lighting, CSM/IBL/TAA post chain,
  vendor upscaling (DLSS/FSR/XeSS) before tonemap. Universally
  supported on every device the engine targets, including the
  RX 580 reference machine (spec §XVIII). The pattern UE5
  shipped on; the pattern every shipping AAA title used through
  2024.
- **Track B — GPU work graphs with mesh shading nodes.** D3D12's
  ExecuteIndirect + work-graph extension (DXR 1.2 era); Vulkan's
  VK_EXT_mesh_shader + VK_NV_device_generated_commands. Cuts
  CPU-side draw submission to near-zero; lets the GPU schedule
  its own workload graph. Hardware support is uneven: every NVIDIA
  Turing+ and AMD RDNA2+ supports mesh shading, but work-graphs
  proper are RDNA3+/Ada-Lovelace+. The RX 580 — spec's reference
  machine — has neither.

The spec (§IV.4.A/B) wants both, but on different timelines: Track A
ships in Phase 5 because the milestone (60 FPS @ 1440p on RX 580)
demands universal hardware support; Track B is research that
becomes default once it matches Track A and exceeds it on
performance.

## Decision

Phase 5 ships Track A as the production renderer:

- Deferred G-buffer + cluster lights (ADR-043) + CSM (ADR-040) +
  IBL (ADR-041) + TAA (ADR-042) + ACES tonemap.
- `UpscalerProvider` trait surface (ADR-005); Phase 5 ships the
  bilinear placeholder and the vendor stubs.
- Frame-pacing milestone gate at Phase 5 PR 6 (ADR-047).

Phase 6+ develops Track B as a research line:

- `engine_render::render_graph::OracleAlternative` (ADR-039) lets
  Track B implementations register as alternative implementations
  of the same render-graph pass. The oracle (ADR-046) is the
  verification mechanism.
- Track B becomes the default backend only when it (a) matches
  Track A pixel-for-pixel on the oracle suite, and (b) beats
  Track A on the frame-pacing bench on every supported GPU.
- Track A is *not removed* when Track B becomes default; it stays
  as the universal fallback for hardware that does not support
  the work-graph extensions. This is the same selection-logic
  pattern ADR-005 uses for upscalers.

The render-graph abstraction (ADR-039) is designed up-front to
support both tracks behind a single contract; PR 1 of Phase 5
delivers the trait surface.

## Rationale

The two-track stance avoids two anti-patterns:

1. **Ship-the-future-only.** Building Track B alone would orphan
   the RX 580 reference milestone, every Intel iGPU laptop, and
   every WebGPU client (work-graphs in WebGPU are not yet
   standardised). The spec's targeting is broader than Track B's
   hardware reach.
2. **Ship-the-past-only.** Building Track A alone makes the
   engine obsolete on a five-year horizon; engines that did not
   plan for mesh-shading/work-graph paths in 2024 are now in
   the middle of expensive retrofits.

The oracle (ADR-046) is the integrating mechanism: Track B's
parity claim is *testable* against Track A, not assertable. A
divergence is a CI failure, not a debate.

## Consequences

- Two pass implementations per visual feature, once Track B
  reaches feature parity (estimated Phase 11–12). The
  maintenance cost is real; the alternative (no future-proofing,
  perpetual catchup work) is worse.
- The render-graph trait (ADR-039) must be expressive enough for
  both styles. Phase 5 PR 1 delivers the surface; if Track B
  reveals that the trait is too Track-A-shaped, an evolution
  ADR will land then. This is anticipated and acceptable.
- The frame-pacing milestone gate (ADR-047) measures Track A.
  Track B will inherit its own milestone gate when it ships.
- The web target (ADR-006) is Track A only for the foreseeable
  future, regardless of when Track B ships natively.

## Risks and tradeoffs

- **The "match Track A pixel-for-pixel" bar is high.** TAA
  history, denoiser temporal patterns, BRDF roundoff — all are
  oracle-tolerance candidates per ADR-046's exception register
  pattern.
- **GPU work-graph standardisation is unfinished.** WebGPU has
  no spec; mobile/console support is uneven. Track B's portability
  envelope will be smaller than Track A's for the foreseeable
  future.
- **Vendor mesh-shader implementations differ.** AMD and NVIDIA
  expose subtly different mesh-shader pipeline-state and
  primitive-output rules. Phase 6+ Track B work absorbs the
  per-vendor adapter cost.
- **Splitting attention.** Two tracks means two test matrices,
  two perf-regression budgets, two oracle suites. The slicing
  ADR-053 keeps Phase-5 attention on Track A; Phase 6+ scheduling
  is a future planning concern.

## Alternatives considered

- **Track A only.** Lower foundation cost; obsolescence cost
  paid later, more expensively. Rejected.
- **Track B only.** Eliminates the legacy maintenance burden;
  fails the RX 580 milestone and the WebGPU constraint.
  Rejected.
- **Pick one and refactor mid-life.** Engines that have done this
  (UE3→UE4 deferred, Unity HDRP introduction) describe it as
  "the worst engineering year of the project's life." Rejected
  before it costs us.
- **Three tracks (forward+, deferred, work-graph).** Two is the
  minimum to cover the architectural split; three multiplies
  maintenance for no additional reach. Rejected.

## Verification

- Track A: Phase 5 PR 6 closes when the frame-pacing milestone
  gate (ADR-047) goes green on the RX 580 self-hosted runner.
- Track B: each future Track B implementation lands behind the
  oracle (ADR-046); the "match Track A pixel-for-pixel" claim
  is a passing oracle run, not a verbal assertion.
- The render-graph trait surface (ADR-039) is verified by the
  Phase 5 PR 1 oracle harness — the same harness Track B will
  use to register `OracleAlternative` implementations later.
- `docs/audit/oracle-exceptions.md` (seeded by Phase 5 PR 6) is
  the running register of acknowledged pixel-level divergences
  between tracks, scoped by GPU and driver version.
