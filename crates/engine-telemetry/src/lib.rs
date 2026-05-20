//! `engine-telemetry` — the telemetry collector, IPC transport, structured
//! logs, and metrics endpoint.
//!
//! Level 1 crate. See `ENGINE_SPECIFICATION_v2.0.md` Part IV.1 and Part X.
//!
//! `engine-core` defines the telemetry *primitives* — the signal types and
//! the per-thread ring buffers game code records into. This crate is the
//! *consumer* side (spec X.4–X.8):
//!
//! - [`collector`] — folds drained signals into metrics and a consent-gated
//!   outbound buffer.
//! - [`ipc`] — the length-prefixed wire protocol external tools speak.
//! - [`log`] — JSON-lines structured logging with size-based rotation.
//! - [`metrics`] — a Prometheus `/metrics` endpoint.
//! - [`consent`] — the opt-in consent gate (ADR-020); nothing leaves the
//!   device until the user grants it.

pub mod collector;
pub mod consent;
pub mod ipc;
pub mod log;
pub mod metrics;
pub mod profiler;

pub use collector::Collector;
pub use consent::ConsentStore;
pub use ipc::{IpcError, Message, WireSignal};
pub use log::{LogLevel, LogRecord, LogWriter};
pub use metrics::MetricsRegistry;
pub use profiler::{FoldedStack, FoldedStacks, SamplingProfiler};
