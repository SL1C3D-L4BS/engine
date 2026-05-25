# ADR-021 — Retain existing LUKS disk layout

- Status: Accepted (deviation from spec §XVIII.3)
- Date: 2026-05-18 (expanded 2026-05-24 per audit §15 Phase-0 ADR sweep)
- Phase: 0 (pre-v1.0 — contract-exempt per risk R-03)
- Companion: spec §XVIII.3 (developer environment disk layout),
  ADR-051 (acknowledged deviations register), `docs/architecture/dev-env.md`
  (future)

## Context

Spec §XVIII.3 prescribes a developer-environment disk layout:

- btrfs as root filesystem (with subvolumes for `@`, `@home`,
  `@log`, `@pkg`).
- XFS as home filesystem (separate partition).
- Snapper-driven snapshots for rollback.

The motivation per spec: btrfs-native snapshots simplify rollback;
XFS on `/home` separates user data lifecycle from the OS.

The current Arch install on the reference workstation was set up
before this spec was written. It uses:

- LUKS full-disk encryption.
- LVM logical volumes on top of the LUKS container.
- A single btrfs volume on the main LV, with subvolumes
  `@`, `@home`, `@log`, `@pkg`.
- No separate XFS partition; `/home` is the `@home` subvolume.

The deviation: a single-btrfs install with LUKS+LVM, rather than
the spec's btrfs-root + XFS-home + (presumably no LUKS) layout.

## Decision

The existing LUKS+LVM+single-btrfs layout is retained. No
reinstall is scheduled.

The deviation is acknowledged in ADR-051; this ADR records the
specific case.

## Rationale

Three reasons for retention:

1. **Existing install is functional.** The system boots, the
   subvolumes work, Snapper rollback works. The spec's intent
   (btrfs root + Snapper rollback) is satisfied.
2. **LUKS adds full-disk encryption the spec omits.** A modern
   developer workstation should have full-disk encryption.
   The deviation is *additive* on this axis.
3. **Reinstalling would destroy the reference library.** The
   workstation has a substantial reference library
   (PDFs / papers / source archives) and live development
   state. The cost of a reinstall — even with backups —
   includes a multi-day recovery window the team cannot
   afford pre-Phase-5.

The XFS-on-`/home` benefit (separating user data lifecycle) is
not currently load-bearing; the single-btrfs setup gives
Snapper rollback for both root and home subvolumes, which is
the property the spec wanted.

## Consequences

- The reference workstation's disk layout differs from the
  spec's prescription.
- Any future contributor following the spec literally will set
  up a different layout; both are documented as acceptable in
  the deviations register.
- Snapper configuration on the existing system is set up per
  the spec's intent (per-subvolume snapshot policy).
- LUKS unlock is part of the boot flow; documented in
  `docs/architecture/dev-env.md` (Phase 10+).

## Risks and tradeoffs

- **Spec-vs-reality drift.** The spec says one thing; reality
  is another. Mitigation: this ADR + ADR-051's register.
- **No XFS-on-/home benefit.** If the team's home subvolume
  grows past btrfs's well-trodden scale (typically
  multi-TB before issues surface), the migration to XFS would
  be costly. Mitigation: at current usage, well under the
  problem zone.
- **LUKS unlock cost** at boot. ~3 seconds on the reference
  hardware; acceptable.
- **LVM layer adds complexity.** LVM tools are mature; the
  team's familiarity with them mitigates the operational cost.

## Alternatives considered

- **Reinstall per spec.** Cost (multi-day recovery) vs. benefit
  (clean conformance) — benefit insufficient.
- **Spec amendment.** Considered; the spec is aspirational and
  the current pragma is a one-off. ADR-051's "deviations as
  living register" pattern handles it cleanly without spec
  edits.
- **Migrate /home to XFS in-place.** Possible (with sufficient
  free space); not scheduled. Could be revisited if a
  btrfs-specific issue emerges.

## Verification

- The reference workstation boots, the engine builds, the
  determinism oracles pass. The disk layout's correctness is
  measured by the engine's existence on it.
- Snapper rollback is exercised periodically (when a system
  update goes wrong). Working as designed.
- The deviations register (ADR-051) includes this entry.
- Documentation: `docs/architecture/dev-env.md` (Phase 10+)
  describes the actual layout for new contributors.
