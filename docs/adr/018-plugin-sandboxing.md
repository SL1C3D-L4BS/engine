# ADR-018 · Plugin sandboxing

- Status: Accepted
- Date: 2026-05-18
- Phase: 0 (pre-v1.0 — contract-exempt per risk R-03)

## Summary

First-party/signed plugins run in-process with full API access. Untrusted plugins run out-of-process with seccomp-bpf and a shared-memory ring buffer; the host survives plugin crashes.

---
*Stub. Full Context / Decision / Rationale / Consequences to be expanded per spec Part XX.2.*
