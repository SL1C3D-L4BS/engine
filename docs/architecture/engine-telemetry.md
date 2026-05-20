# engine-telemetry

The telemetry collector, IPC transport, structured logs, and metrics endpoint
(spec IV.1 Level 1, X.4–X.8).

## Purpose

`engine-core` defines the telemetry *primitives* — the signal types and the
per-thread ring buffers game code records into. This crate is the *consumer*
side: it drains those signals and exposes them to external tools.

## Modules

| Module      | Contents |
|-------------|----------|
| `collector` | `Collector` — folds drained signals into the metrics registry and a consent-gated outbound buffer; the single point where the consent gate is enforced. |
| `ipc`       | The length-prefixed wire protocol (`Message` types `0x01`–`0x0A`) external tools speak over a Unix-domain socket; `WireSignal` is the owned form of a signal. |
| `log`       | `LogWriter` — JSON-lines structured logging with size-based rotation; `Trace`/`Debug` records are dropped in release builds. |
| `metrics`   | `MetricsRegistry` and a tiny `/metrics` HTTP responder in Prometheus text format. |
| `consent`   | `ConsentStore` — the opt-in consent gate (ADR-020). |

## Design notes

- The IPC body encoding is *owned* compact binary rather than MessagePack: the
  message set is small, fixed, and versioned with the engine, so owning it
  removes a dependency and a class of schema-drift bugs (R-02).
- Metrics aggregation is local and always runs; buffering signals for
  off-process delivery happens **only** after consent is granted, and
  pre-consent signals are never retained.
- Log levels `Trace` and `Debug` are stripped in release builds — verbose
  logging must not cost a shipped game anything.

## Oracle

- `ipc` unit tests round-trip every `Message` type through encode → decode.
- `log` unit tests validate the JSON-lines schema, escaping, and rotation.
- `tests/oracle.rs` runs the full path: a signal recorded into the
  `engine-core` ring survives collection, IPC encoding, and decoding unchanged
  — and the consent gate denies emission until granted.

## Dependencies

`engine-core`, `engine-platform` — Level 1.
