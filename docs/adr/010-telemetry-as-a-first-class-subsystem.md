# ADR-010 — Telemetry as a first-class subsystem

- Status: Accepted
- Date: 2026-05-18 (expanded 2026-05-24 per audit §15 Phase-0 ADR sweep)
- Phase: 0 (pre-v1.0 — contract-exempt per risk R-03)
- Companion: ADR-020 (telemetry consent is opt-in), ADR-030 (owned
  sampling profiler — the per-thread side), ADR-051 (acknowledged
  deviations — owned binary IPC instead of MessagePack), spec §X
  (Telemetry)

## Context

The engine's runtime has dozens of subsystems producing diagnostic
information: scheduler frame digests, GC pauses, asset load
events, shader compile events, profiler samples, network packet
loss counters. Without a unified telemetry substrate, every
subsystem grows its own `eprintln!`-style channel, every tool
that wants to consume diagnostic data parses a different ad-hoc
format, and per-tool integrations multiply.

The spec (§X) elevates telemetry to a first-class subsystem:
one IPC channel, one wire format, every tool reads the same
announcements. The substrate is the foundation for everything
serious downstream — the profiler GUI (Phase 10), the editor's
diagnostics panel (Phase 10), the crash-handler postmortem
(Phase 10, ADR-011), the engine-tui live introspector.

## Decision

`engine-telemetry` is a Level-0 crate (foundation layer). Its
contract:

- **IPC framing:** binary, fixed-header
  `[frame_len:u32 | msg_type:u8 | flags:u8 | seq:u16]` (spec §X.5),
  body as owned compact little-endian binary encoding (the body
  encoding is the owned deviation acknowledged in ADR-051; the
  spec called for MessagePack).
- **Transport:** Unix domain socket on Linux/macOS; named pipe on
  Windows. Discovery via a well-known filesystem path under
  `$XDG_RUNTIME_DIR` (Linux) / `%TEMP%` (Windows).
- **One channel.** Every subsystem publishes to the same socket;
  every consumer subscribes to the same socket. The message-type
  byte demuxes.
- **Consent-gated.** No telemetry leaves the device until consent
  is recorded (ADR-020). Without consent, the IPC channel
  publishes locally for in-process tools but no upload occurs.
- **Owned encoder.** The IPC body encoder is a ~150-line owned
  binary encoder per ADR-051; serde is not in the engine's
  foundation-layer dependency set.

The message-type registry is owned by `engine_telemetry::types`
and edited by PR: adding a new event is a deliberate decision
visible in code review.

## Rationale

A unified telemetry substrate has three benefits no per-subsystem
channel can achieve:

1. **Tool fan-out is constant.** A new tool (a frame-pacing
   live monitor, a memory-leak hunter) subscribes to the same
   socket and demuxes by type. Adding a tool is not adding a
   parser.
2. **Correlation is automatic.** Two events (a scheduler frame
   tick + a GC pause) share a timeline because they're on the
   same socket; tools can interleave by sequence number.
3. **Postmortem is a recording.** The crash handler (ADR-011)
   writes the last N seconds of telemetry to the crash buffer;
   postmortem replay is a deterministic event stream.

The owned-binary-body deviation (ADR-051) was a deliberate
foundation-layer simplification: the bodies are small, the gain
from MessagePack would be ~10–20%, and adding serde to the
foundation pulls a substantial transitive dependency tree the
audit-and-oracle stance discourages. The frame header is owned
either way (per spec).

## Consequences

- `engine_telemetry` is in the foundation layer; it has zero
  third-party dependencies (no serde, no MessagePack).
- Every subsystem that wants to publish telemetry depends on
  `engine_telemetry` directly. The dep graph is shallow.
- Tools (`engine-tui`, future `engine-postmortem`, future
  `engine-profiler-gui`) speak the IPC protocol; the engine
  ships the protocol decoder as a library.
- Consent (ADR-020) is checked in the *upload* path, not the
  *publish* path. Local diagnostic tooling works regardless of
  consent; only network egress is gated.
- The IPC body encoder is small, owned, and oracle-tested.

## Risks and tradeoffs

- **Owned encoder is the engine's, not the world's.** External
  consumers (e.g. a future Hetzner Prometheus exporter) would
  need to use the engine's decoder library or learn the wire
  format. ADR-051's gate condition: if an external consumer
  arrives, MessagePack returns.
- **Schema evolution.** Adding a field to an existing message
  type is non-breaking; renaming or removing requires a
  versioned message-type bump. The registry has a
  schema-version field. Routine evolution.
- **Buffer back-pressure.** A slow consumer must not block the
  engine. Mitigation: the publisher uses a bounded ring buffer
  with a drop-oldest policy on overflow; drops are themselves
  reported via a `Dropped` event.
- **One-socket-everywhere** discoverability fails if two engine
  instances run on the same machine. Mitigation: socket path
  includes engine PID; tools enumerate by directory listing.

## Alternatives considered

- **No telemetry framework; subsystems use logging.** Standard
  early-engine choice; doesn't scale to dozens of subsystems
  and external tools. Rejected.
- **MessagePack via `rmp-serde`.** Spec's original choice; owns
  the audit-deferred deviation per ADR-051. Re-evaluate per
  gate condition.
- **OpenTelemetry / OTLP.** Industry-standard observability;
  binary size and dependency surface unsuitable for the
  foundation layer. Possible Phase 10+ exporter, not a
  substrate replacement.
- **Per-subsystem logging via `tracing`.** The `tracing` crate
  is excellent; its consumers are heterogeneous and its wire
  formats vary. The engine's needs are narrower and more
  unified. Rejected as the substrate; remains available as a
  per-crate adjunct if needed.

## Verification

- `cargo test -p engine-telemetry` — round-trip encoder
  oracle (every message type encodes + decodes lossless).
- IPC unit tests with mock sockets: publish events from one
  thread, subscribe from another, verify ordering and content.
- The frame-pacing benchmark (ADR-047) emits telemetry events
  that the bench binary reads back to assemble the JSON
  report; this is the dogfood verification path.
- Consent gate (ADR-020) tested by setting the consent flag
  to false and verifying that no network egress occurs even
  with active subscribers.
- The owned-binary IPC deviation (ADR-051) is verified by a
  round-trip oracle and acknowledged in the deviation register.
