//! Cross-module oracle for `engine-telemetry`: a signal recorded into the
//! `engine-core` ring must survive collection, consent gating, IPC encoding,
//! and decoding back to its original value (spec X.4–X.8).

use engine_core::telemetry::{self, Signal, Subsystem};
use engine_telemetry::ipc::{decode, encode};
use engine_telemetry::{Collector, Message, WireSignal};

#[test]
fn signal_survives_the_full_collect_encode_decode_path() {
    let _ = telemetry::drain_local(); // clear residue

    telemetry::record(Signal::Gauge {
        name: "frame_ms",
        subsystem: Subsystem::Render,
        value: 16.6,
        unit: "ms",
    });

    // Consent granted: the collector buffers the signal for delivery.
    let mut collector = Collector::new(true);
    collector.collect_local();

    let outbound = collector.drain_outbound();
    let Some(Message::Signals(signals)) = outbound.first() else {
        panic!("expected a Signals message, got {outbound:?}");
    };
    assert_eq!(
        signals[0],
        WireSignal::Gauge {
            name: "frame_ms".into(),
            subsystem: Subsystem::Render.as_u8(),
            value: 16.6,
            unit: "ms".into(),
        }
    );

    // The message round-trips through the wire codec unchanged.
    let frame = encode(&outbound[0]);
    let (decoded, consumed) = decode(&frame).expect("frame decodes");
    assert_eq!(decoded, outbound[0]);
    assert_eq!(consumed, frame.len());
}

#[test]
fn consent_gate_denies_emission_until_granted() {
    let _ = telemetry::drain_local();
    telemetry::record(Signal::Counter {
        name: "denied",
        subsystem: Subsystem::Telemetry,
        increment: 1,
    });

    // No consent: the signal is aggregated locally but never buffered to send.
    let mut collector = Collector::new(false);
    collector.collect_local();
    assert_eq!(collector.metrics().counter("denied"), Some(1));
    assert!(collector.drain_outbound().is_empty());
}
