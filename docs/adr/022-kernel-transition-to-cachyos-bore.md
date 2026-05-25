# ADR-022 — Kernel transition to cachyos-bore

- Status: Accepted (deviation from spec §XVIII.4)
- Date: 2026-05-18 (expanded 2026-05-24 per audit §15 Phase-0 ADR sweep)
- Phase: 0 (pre-v1.0 — contract-exempt per risk R-03)
- Companion: spec §XVIII.4 (developer environment kernel),
  ADR-051 (acknowledged deviations register), ADR-021 (disk
  layout)

## Context

Spec §XVIII.4 names `linux-zen` as the kernel for the developer
environment. Zen-kernel ships with desktop-tuned scheduler
patches and is the standard Arch alternative kernel for gaming /
desktop workloads.

The CachyOS project ships `linux-cachyos-bore`, which combines:

- The BORE scheduler (Burst-Oriented Response Enhancement), an
  evolution of CFS designed to favour interactive bursty
  workloads — a better fit for the engine's mix (editor +
  build + game run).
- Zen-kernel-derived patches (most of zen's perf wins).
- LTO-compiled kernel binaries.

Empirically on the reference workstation, `linux-cachyos-bore`
gave perceptibly lower latency under heavy load (build +
running engine + browser + reference docs open simultaneously)
than `linux-zen` on the same hardware.

## Decision

The reference workstation's default boot kernel is
`linux-cachyos-bore`. `linux-zen` is retained as a fallback
systemd-boot/UKI entry — not removed — to guarantee a
known-good boot path during Phase 0.

The deviation is acknowledged in ADR-051; this ADR records
the specific case.

## Rationale

Three reasons:

1. **Measurable latency improvement** under the team's typical
   workload (interactive editing + builds + engine running).
   BORE's bursty-favouring policy fits the engine's
   "compile, run, debug, iterate" inner loop.
2. **CachyOS is actively maintained.** Updates land within
   days of upstream mainline kernel releases; the project's
   pace exceeds linux-zen's.
3. **Fallback retained.** The risk of a CachyOS-specific
   regression breaking the dev environment is mitigated by
   keeping `linux-zen` as a fallback boot entry. Worst case:
   one reboot.

The spec's `linux-zen` recommendation predates the team's
direct experience with BORE; the deviation is an evolutionary
refinement, not a contradiction of the spec's intent.

## Consequences

- The reference workstation boots `linux-cachyos-bore` by
  default.
- `linux-zen` remains installed (not removed) for fallback.
- The fallback is documented in `docs/architecture/dev-env.md`
  (Phase 10+) so contributors can recover if a CachyOS update
  breaks the system.
- No engine code depends on kernel-specific behaviour; the
  engine runs on any modern Linux kernel (5.15+).
- The frame-pacing CI gate (ADR-047) runs on the self-hosted
  RX 6700 XT runner; that runner's kernel is independently
  chosen for measurement stability (Linux LTS for the runner).
  The dev workstation's kernel choice does not affect CI.

## Risks and tradeoffs

- **CachyOS upstream maintenance** is a smaller-team
  project than mainline Arch. Mitigation: fallback boot
  entry; the LTS kernel is also installable if both
  cachyos-bore and zen fail.
- **BORE scheduler edge cases** are less battle-tested than
  CFS / EEVDF. Mitigation: rolling back the kernel is a one-
  reboot fix; no engine state depends on the scheduler.
- **Driver/module sync.** NVIDIA modules build against the
  kernel headers; `linux-cachyos-bore` builds them
  successfully but the build cadence is faster than zen's.
  Mitigation: `dkms` handles the auto-rebuild; documented.

## Alternatives considered

- **Stay on linux-zen.** Misses the BORE latency win;
  acceptable but suboptimal.
- **linux-mainline.** No desktop-tuning patches; less
  responsive for the team's workload.
- **linux-lts.** Most stable; least responsive. Suitable for
  the self-hosted CI runner but not the dev workstation.
- **Custom-compile a kernel.** Owned discipline taken to an
  extreme; not justified for an OS layer.

## Verification

- The reference workstation boots `linux-cachyos-bore`,
  builds the engine, runs the determinism oracles.
- The fallback `linux-zen` entry is reachable via
  systemd-boot's menu.
- Documentation (`docs/architecture/dev-env.md`, Phase 10+)
  describes the recovery path.
- The deviations register (ADR-051) includes this entry.
- No CI gate; kernel choice is a developer-environment
  recommendation, not a build-time contract.
