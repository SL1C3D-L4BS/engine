# ADR-018 — Plugin sandboxing

- Status: Accepted
- Date: 2026-05-18 (expanded 2026-05-24 per audit §15 Phase-0 ADR sweep)
- Phase: 0 (pre-v1.0 — contract-exempt per risk R-03)
- Companion: ADR-019 (asset sandbox subprocesses — narrower
  case of the same pattern), ADR-017 (Game Master provider —
  often runs as a plugin), spec §XVII, spec §XIX.2 (security)

## Context

The engine ships a plugin system. The 2026 archetypes:

- Asset importers (FBX, OBJ, animation, audio formats — beyond
  the engine's owned formats).
- AI providers (the Game Master per ADR-017).
- Anti-cheat hooks (per-genre; ADR-009).
- Editor extensions (custom tooling per project).
- Live Ops integrations (analytics exporters, content delivery,
  player support).

These plugins fall into two trust tiers:

- **First-party or signed plugins.** Code reviewed, signed by a
  trusted key, intended to run in-process with full API access.
  Examples: the engine's own asset importers, vetted
  community plugins.
- **Untrusted plugins.** Code from unknown or unvetted sources.
  Examples: a third-party Discord-integration plugin, a
  user-installed importer for an exotic format.

A plugin model that runs all plugins in-process gives untrusted
code the same memory and network access as the engine — a
catastrophic security posture. A plugin model that runs every
plugin in a subprocess pays unacceptable latency for trusted
plugins on the hot path.

## Decision

Plugin sandboxing has two modes selected per plugin:

### Mode 1: In-process (first-party / signed)

- The plugin is a dynamically loaded shared object
  (`libfoo.so` / `foo.dll` / `foo.dylib`).
- The plugin binds against `engine-plugin-api` (the contract
  crate; today empty per Phase-0 status, opens for Phase 10+).
- Signature verification: an Ed25519 signature on the
  shared-object bytes is checked against a trusted-key set
  before loading. Unsigned plugins are rejected at load time.
- Crashes in the plugin crash the engine (no isolation).
  Acceptable for first-party code; the audit trail is the
  signature.

### Mode 2: Out-of-process (untrusted)

- The plugin runs as a separate subprocess, communicating with
  the engine via a Unix domain socket / named pipe.
- **seccomp-bpf** (Linux) / job-object restrictions (Windows)
  apply a syscall allowlist: no network unless explicitly
  permitted by capabilities; no filesystem access outside a
  per-plugin sandbox directory; no shared memory beyond the
  ring buffer.
- **Shared-memory ring buffer** for high-throughput data
  exchange (asset bytes, telemetry events). The ring buffer is
  mapped read-only on the plugin side for outbound data,
  read-write for inbound; the engine survives any
  ring-buffer-corruption attempt.
- Crashes in the plugin are isolated; the engine catches the
  SIGCHLD / process-exit event and reports it via telemetry
  (ADR-010).

Per-plugin **capabilities** declare what the plugin is allowed
to do (network access, filesystem scopes, GPU access). The
engine enforces; the plugin manifest declares.

## Rationale

Two-tier sandboxing is the standard pattern (Chrome's
renderer/main split, VS Code's extensions, browser extension
manifests). The engine's adaptation:

1. **In-process for trusted** keeps the hot path fast. Asset
   importers running once per game-load can afford a subprocess
   round-trip; GM providers (ADR-017) on the per-request path
   often cannot.
2. **Out-of-process for untrusted** is the only acceptable
   security posture for code the engine does not vet.
3. **Capabilities (instead of full sandbox-or-nothing)** lets
   per-plugin policy be expressive without complicating the
   common case.

The pattern aligns with ADR-019 (asset sandbox subprocesses) —
which is itself a specialised instance of the out-of-process
tier applied to file-format parsers.

## Consequences

- `engine-plugin-api` is the contract crate; Phase 10+
  expands it.
- The plugin loader (Phase 10+) implements both modes; the
  signature verification path lands in Phase 10 alongside the
  editor's plugin manager UI.
- A plugin's capabilities are declared in a `plugin.toml` /
  manifest that the loader parses before instantiating the
  plugin.
- The seccomp-bpf filter is a per-platform implementation; the
  Linux baseline matches Chrome's renderer filter shape
  (whitelist of safe syscalls). Windows uses job objects.
  macOS uses sandbox-exec / endpoint security framework.
- The engine survives plugin crashes when sandboxed; this is
  a verification target for the Phase 10 implementation.

## Risks and tradeoffs

- **seccomp / job-object discipline is fragile.** A new syscall
  the filter doesn't recognise breaks the plugin. Mitigation:
  the per-platform filter file is versioned; new syscalls
  added explicitly.
- **Performance cost of out-of-process.** A subprocess
  round-trip is in the 100s of µs even with shared memory.
  Mitigation: hot-path plugins are in-process (trusted);
  cold-path plugins are out-of-process (untrusted).
- **Cross-platform sandboxing parity.** Linux's seccomp,
  Windows's job-object, macOS's sandbox-exec differ. The
  engine's capabilities surface is the unified API; per-platform
  enforcement is a per-platform implementation cost.
- **Plugin author UX.** Two modes is more complex than one.
  Mitigation: `cargo new --template engine-plugin-untrusted`
  / `--template engine-plugin-signed` (Phase 10+) scaffolds the
  right setup.

## Alternatives considered

- **One-tier in-process.** No untrusted-code story; only
  signed plugins. Loses the third-party ecosystem story.
  Rejected.
- **One-tier out-of-process.** Pays subprocess cost for every
  plugin including trusted ones. Rejected.
- **WASM-based plugins.** Solid sandbox; loses the C++/Rust/Go
  plugin ecosystem (GGM provider implementations would need to
  ship llama.cpp inside a WASM module — currently infeasible).
  Phase 11+ candidate; not Phase 0.
- **In-process with `setrlimit` / cgroups isolation.** Limits
  some resources; does not prevent malicious code from
  exfiltrating data, mounting attacks, etc. Insufficient.

## Verification

- Phase 10 ships the plugin loader + signature verification +
  seccomp-bpf filter. Integration test corpus:
  - In-process: a signed test plugin loads, runs, and unloads
    without affecting engine state.
  - Out-of-process: a deliberately misbehaving test plugin
    (tries to make a network call, tries to access the
    filesystem outside its sandbox) is killed by the kernel /
    fails the capability check.
  - Crash isolation: an out-of-process plugin that SIGSEGVs
    does not bring down the engine.
- Audit: every shipped first-party plugin must be signed by
  the engine's release key.
- The `engine-plugin-api`'s contract is owned by ADR-012 (50-
  year API stability); semver-checks runs on it.
