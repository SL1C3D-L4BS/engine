# Reproducible-build log

Spec §XX.8 demands the engine be bit-identical across two cold-cache
builds of the same commit. ADR-052 specifies the verification cadence
(weekly Sunday 02:00 UTC) and method (two-target-dir SHA-256
comparison) implemented in
`.github/workflows/reproducible-build.yml`.

This log accumulates the result of each scheduled run. The CI workflow
uploads a per-run artefact (`reproducible-build-<run-id>`) containing
the hash list and diff; this document is the monthly human-readable
roll-up.

## Status

Workflow installed: 2026-05-24 (audit remediation packet).
First scheduled run: next Sunday after merge.

## Entries

(First entry appended after the first scheduled run.)
