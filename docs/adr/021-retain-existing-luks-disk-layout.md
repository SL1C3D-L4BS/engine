# ADR-021 · Retain existing LUKS disk layout

- Status: Accepted
- Date: 2026-05-18
- Phase: 0 (pre-v1.0 — contract-exempt per risk R-03)

## Summary

DEVIATION from spec XVIII.3. Arch is already installed on LUKS -> LVM -> single btrfs (subvols @ @home @log @pkg) rather than the spec's btrfs-root + XFS-home layout. Kept: the install is functional, adds full-disk encryption the spec omits, and reinstalling would destroy the reference library. The spec intent (btrfs root + Snapper rollback) is satisfied.

---
*Stub. Full Context / Decision / Rationale / Consequences to be expanded per spec Part XX.2.*
