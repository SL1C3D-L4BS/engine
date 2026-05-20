//! The telemetry collector.
//!
//! Game threads record signals into their own thread-local rings (the
//! `engine-core` telemetry primitives). The collector is the consumer: it
//! folds drained signals into the [`MetricsRegistry`] and buffers them for the
//! IPC transport.
//!
//! The collector is the single point where the **consent gate** (ADR-020) is
//! enforced. Metrics aggregation is a local, in-process activity and always
//! runs; buffering signals for off-process delivery happens *only* once
//! consent has been granted — without it, [`drain_outbound`](Collector::drain_outbound)
//! has nothing to send and pre-consent signals are never retained.

use crate::ipc::{Message, WireSignal};
use crate::metrics::MetricsRegistry;
use engine_core::telemetry::{self, Signal};
use std::time::Duration;

/// Consumes telemetry signals into local metrics and a consent-gated outbound
/// buffer.
#[derive(Debug)]
pub struct Collector {
    metrics: MetricsRegistry,
    consent_granted: bool,
    pending: Vec<WireSignal>,
    dropped: u64,
}

impl Collector {
    /// Creates a collector. `consent_granted` is the result of the
    /// [`consent`](crate::consent) gate at startup.
    pub fn new(consent_granted: bool) -> Self {
        Self {
            metrics: MetricsRegistry::new(),
            consent_granted,
            pending: Vec::new(),
            dropped: 0,
        }
    }

    /// Whether off-process delivery is permitted.
    pub fn consent_granted(&self) -> bool {
        self.consent_granted
    }

    /// Folds a batch of signals in: metrics are always updated; the signals
    /// are buffered for IPC only if consent has been granted.
    pub fn ingest(&mut self, signals: &[Signal]) {
        for signal in signals {
            self.metrics.ingest(signal);
            if self.consent_granted {
                self.pending.push(WireSignal::from_signal(signal));
            }
        }
    }

    /// Drains the calling thread's telemetry ring and ingests it, accounting
    /// for any signals the ring dropped to overflow.
    pub fn collect_local(&mut self) {
        let signals = telemetry::drain_local();
        self.dropped = telemetry::overflow_count();
        self.ingest(&signals);
    }

    /// The live metrics registry, for the `/metrics` endpoint.
    pub fn metrics(&self) -> &MetricsRegistry {
        &self.metrics
    }

    /// An independent copy of the metrics, e.g. for a `/metrics` scrape served
    /// on another thread.
    pub fn metrics_snapshot(&self) -> MetricsRegistry {
        self.metrics.clone()
    }

    /// Number of signals the ring dropped to overflow at the last
    /// [`collect_local`](Self::collect_local).
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Takes the buffered signals as IPC messages, clearing the buffer.
    ///
    /// Returns an empty vector when consent has not been granted — the heart
    /// of the consent gate: no signal can leave the process without it.
    pub fn drain_outbound(&mut self) -> Vec<Message> {
        if !self.consent_granted || self.pending.is_empty() {
            self.pending.clear();
            return Vec::new();
        }
        let mut messages = vec![Message::Signals(std::mem::take(&mut self.pending))];
        if self.dropped > 0 {
            messages.push(Message::DropReport {
                dropped: self.dropped,
            });
        }
        messages
    }

    /// Number of signals currently buffered for outbound delivery.
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }
}

/// The collector's intended drain cadence (spec X.4): once per millisecond.
pub const DRAIN_INTERVAL: Duration = Duration::from_millis(1);

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::telemetry::Subsystem;

    fn counter(name: &'static str, n: u64) -> Signal {
        Signal::Counter {
            name,
            subsystem: Subsystem::Telemetry,
            increment: n,
        }
    }

    #[test]
    fn metrics_aggregate_regardless_of_consent() {
        let mut collector = Collector::new(false);
        collector.ingest(&[counter("ticks", 4), counter("ticks", 6)]);
        assert_eq!(collector.metrics().counter("ticks"), Some(10));
    }

    #[test]
    fn consent_gate_blocks_outbound_emission() {
        let mut collector = Collector::new(false);
        collector.ingest(&[counter("ticks", 1)]);
        // No consent: nothing buffered, nothing emitted.
        assert_eq!(collector.pending_count(), 0);
        assert!(collector.drain_outbound().is_empty());
    }

    #[test]
    fn granted_consent_allows_outbound_emission() {
        let mut collector = Collector::new(true);
        collector.ingest(&[counter("ticks", 1), counter("ticks", 1)]);
        assert_eq!(collector.pending_count(), 2);

        let messages = collector.drain_outbound();
        assert_eq!(messages.len(), 1);
        assert!(matches!(&messages[0], Message::Signals(s) if s.len() == 2));
        // The buffer is emptied by the drain.
        assert_eq!(collector.pending_count(), 0);
    }

    #[test]
    fn collect_local_drains_the_thread_ring() {
        let _ = telemetry::drain_local(); // clear residue from other tests
        telemetry::record(counter("local", 3));
        let mut collector = Collector::new(false);
        collector.collect_local();
        assert_eq!(collector.metrics().counter("local"), Some(3));
    }
}
