//! Telemetry primitives.
//!
//! Observability is built in, not bolted on (spec I.1). This module defines
//! the four owned signal types and the per-thread, loss-tolerant ring buffer
//! they are recorded into (spec X.1–X.3). The collector, IPC transport, and
//! structured logs that *consume* these signals live in the `engine-telemetry`
//! crate.
//!
//! Recording is cheap and never blocks game code: a full ring drops its
//! oldest entry rather than stalling the producer.

use crate::alloc::RingArena;
use std::cell::RefCell;
use std::sync::OnceLock;
use std::time::Instant;

/// The subsystem a signal originated from (spec X.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Subsystem {
    /// Entity Component System.
    Ecs,
    /// Rendering.
    Render,
    /// Physics.
    Physics,
    /// Audio.
    Audio,
    /// Networking.
    Net,
    /// Scripting VM.
    Script,
    /// Artificial intelligence.
    Ai,
    /// Asset pipeline.
    Asset,
    /// Editor.
    Editor,
    /// Hub.
    Hub,
    /// Platform / OS abstraction.
    Platform,
    /// Telemetry itself.
    Telemetry,
}

impl Subsystem {
    /// The subsystem's numeric tag.
    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

/// One telemetry record (spec X.1).
#[derive(Clone, Debug, PartialEq)]
pub enum Signal {
    /// A named, timed operation.
    Span {
        /// Operation name.
        name: &'static str,
        /// Originating subsystem.
        subsystem: Subsystem,
        /// Start timestamp, nanoseconds since process start.
        start_ns: u64,
        /// End timestamp, nanoseconds since process start.
        end_ns: u64,
    },
    /// A monotonic counter increment.
    Counter {
        /// Counter name.
        name: &'static str,
        /// Originating subsystem.
        subsystem: Subsystem,
        /// Amount added.
        increment: u64,
    },
    /// A point-in-time measurement.
    Gauge {
        /// Gauge name.
        name: &'static str,
        /// Originating subsystem.
        subsystem: Subsystem,
        /// Measured value.
        value: f64,
        /// Unit label (e.g. `"bytes"`, `"ms"`).
        unit: &'static str,
    },
    /// A discrete occurrence with a flat key/value payload.
    Event {
        /// Event name.
        name: &'static str,
        /// Originating subsystem.
        subsystem: Subsystem,
        /// Key/value fields.
        fields: Vec<(String, String)>,
    },
}

/// Slots in each thread's telemetry ring (spec X.3).
pub const RING_CAPACITY: usize = 65_536;

fn epoch() -> Instant {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    *EPOCH.get_or_init(Instant::now)
}

/// Monotonic nanoseconds since the first telemetry call in this process.
pub fn now_ns() -> u64 {
    epoch().elapsed().as_nanos() as u64
}

thread_local! {
    static RING: RefCell<RingArena<Signal>> =
        RefCell::new(RingArena::with_capacity(RING_CAPACITY));
}

/// Records a signal into the calling thread's ring buffer.
///
/// If the ring is full the oldest signal is dropped — telemetry is
/// loss-tolerant and must never block the caller.
pub fn record(signal: Signal) {
    RING.with(|ring| {
        ring.borrow_mut().push(signal);
    });
}

/// Drains every signal recorded on the calling thread since the last drain.
pub fn drain_local() -> Vec<Signal> {
    RING.with(|ring| ring.borrow_mut().drain())
}

/// The number of signals dropped by ring overflow on the calling thread.
pub fn overflow_count() -> u64 {
    RING.with(|ring| ring.borrow().dropped())
}

/// Times a block and records it as a [`Signal::Span`]. Evaluates to the
/// block's value.
#[macro_export]
macro_rules! span {
    ($name:expr, $subsystem:expr, $body:block) => {{
        let __span_start = $crate::telemetry::now_ns();
        let __span_result = $body;
        $crate::telemetry::record($crate::telemetry::Signal::Span {
            name: $name,
            subsystem: $subsystem,
            start_ns: __span_start,
            end_ns: $crate::telemetry::now_ns(),
        });
        __span_result
    }};
}

/// Records a [`Signal::Counter`] increment.
#[macro_export]
macro_rules! counter_inc {
    ($name:expr, $subsystem:expr, $amount:expr) => {
        $crate::telemetry::record($crate::telemetry::Signal::Counter {
            name: $name,
            subsystem: $subsystem,
            increment: $amount,
        })
    };
}

/// Records a [`Signal::Gauge`] sample.
#[macro_export]
macro_rules! gauge_set {
    ($name:expr, $subsystem:expr, $value:expr, $unit:expr) => {
        $crate::telemetry::record($crate::telemetry::Signal::Gauge {
            name: $name,
            subsystem: $subsystem,
            value: $value,
            unit: $unit,
        })
    };
}

/// Records a [`Signal::Event`] with a flat key/value payload.
#[macro_export]
macro_rules! event {
    ($name:expr, $subsystem:expr, { $($key:expr => $value:expr),* $(,)? }) => {
        $crate::telemetry::record($crate::telemetry::Signal::Event {
            name: $name,
            subsystem: $subsystem,
            fields: ::std::vec![ $( (::std::string::ToString::to_string(&$key), ::std::format!("{}", $value)) ),* ],
        })
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_and_drain() {
        // Drain anything left by other tests on this thread first.
        let _ = drain_local();
        record(Signal::Counter {
            name: "frames",
            subsystem: Subsystem::Ecs,
            increment: 1,
        });
        let drained = drain_local();
        assert_eq!(drained.len(), 1);
        assert!(drain_local().is_empty());
    }

    #[test]
    fn macros_record_each_signal_kind() {
        let _ = drain_local();
        let doubled = span!("work", Subsystem::Render, { 21 * 2 });
        assert_eq!(doubled, 42);
        counter_inc!("draws", Subsystem::Render, 7);
        gauge_set!("vram", Subsystem::Render, 1024.0, "bytes");
        event!("hot_reload", Subsystem::Asset, { "path" => "a.bp", "ms" => 3 });

        let drained = drain_local();
        assert_eq!(drained.len(), 4);
        assert!(matches!(drained[0], Signal::Span { name: "work", .. }));
        assert!(matches!(
            drained[3],
            Signal::Event {
                name: "hot_reload",
                ..
            }
        ));
    }

    #[test]
    fn subsystem_tag_is_a_small_integer() {
        assert_eq!(Subsystem::Ecs.as_u8(), 0);
        assert_eq!(Subsystem::Telemetry.as_u8(), 11);
    }
}
