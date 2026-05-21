//! Prometheus metrics exposition.
//!
//! When `ENGINE_METRICS=1` the engine exposes a `/metrics` endpoint (on the
//! `metrics_port` from `engine.toml`, default `9091`) that a Prometheus
//! scraper reads. This module owns the registry that aggregates telemetry
//! signals into metrics, the Prometheus text rendering, and a deliberately
//! tiny HTTP responder — the endpoint serves exactly one route, so a full
//! HTTP stack would be unjustified weight (R-02).

use engine_core::telemetry::Signal;
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, ToSocketAddrs};

/// Returns whether the metrics endpoint is enabled for this process.
pub fn metrics_enabled() -> bool {
    std::env::var("ENGINE_METRICS").is_ok_and(|v| v == "1")
}

/// Aggregates telemetry signals into scrapeable metrics.
///
/// Ordering is deterministic — both maps are sorted — so the rendered output
/// is stable across runs.
#[derive(Clone, Debug, Default)]
pub struct MetricsRegistry {
    counters: BTreeMap<String, u64>,
    gauges: BTreeMap<String, f64>,
}

impl MetricsRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Folds one telemetry [`Signal`] into the registry.
    pub fn ingest(&mut self, signal: &Signal) {
        match signal {
            Signal::Counter {
                name, increment, ..
            } => {
                *self.counters.entry(metric_name(name)).or_insert(0) += increment;
            }
            Signal::Gauge { name, value, .. } => {
                self.gauges.insert(metric_name(name), *value);
            }
            Signal::Span {
                name,
                start_ns,
                end_ns,
                ..
            } => {
                *self
                    .counters
                    .entry(format!("{}_count", metric_name(name)))
                    .or_insert(0) += 1;
                self.gauges.insert(
                    format!("{}_last_ns", metric_name(name)),
                    end_ns.saturating_sub(*start_ns) as f64,
                );
            }
            Signal::Event { name, .. } => {
                *self
                    .counters
                    .entry(format!("{}_total", metric_name(name)))
                    .or_insert(0) += 1;
            }
            Signal::Sample { count, .. } => {
                // Aggregate every observed call-chain into one global
                // counter — the per-stack identity lives in the IPC
                // channel for tools that consume folded stacks
                // (ADR-030); the metrics endpoint only needs the
                // aggregate throughput.
                *self
                    .counters
                    .entry("sampling_profiler_samples_total".to_string())
                    .or_insert(0) += count;
            }
            Signal::ScriptBreakpointHit { .. } => {
                *self
                    .counters
                    .entry("sli_script_breakpoint_hits_total".to_string())
                    .or_insert(0) += 1;
            }
            Signal::ScriptException { .. } => {
                *self
                    .counters
                    .entry("sli_script_exceptions_total".to_string())
                    .or_insert(0) += 1;
            }
        }
    }

    /// The current value of a counter, if present.
    pub fn counter(&self, name: &str) -> Option<u64> {
        self.counters.get(name).copied()
    }

    /// The current value of a gauge, if present.
    pub fn gauge(&self, name: &str) -> Option<f64> {
        self.gauges.get(name).copied()
    }

    /// Renders the registry in the Prometheus text exposition format.
    pub fn render(&self) -> String {
        let mut out = String::new();
        for (name, value) in &self.counters {
            out.push_str(&format!("# TYPE {name} counter\n{name} {value}\n"));
        }
        for (name, value) in &self.gauges {
            out.push_str(&format!("# TYPE {name} gauge\n{name} {value}\n"));
        }
        out
    }
}

/// Sanitizes an arbitrary signal name into a valid Prometheus metric name:
/// any character outside `[a-zA-Z0-9_:]` becomes `_`.
fn metric_name(raw: &str) -> String {
    let mut name: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == ':' {
                c
            } else {
                '_'
            }
        })
        .collect();
    // A metric name may not start with a digit.
    if name.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        name.insert(0, '_');
    }
    name
}

/// Builds the full HTTP response for a `/metrics` scrape of `registry`.
pub fn http_response(registry: &MetricsRegistry) -> String {
    let body = registry.render();
    format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: text/plain; version=0.0.4\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {body}",
        body.len(),
    )
}

/// The fixed 404 response for any route other than `/metrics`.
pub fn not_found_response() -> String {
    let body = "not found";
    format!(
        "HTTP/1.1 404 Not Found\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {body}",
        body.len(),
    )
}

/// `true` if `request` is a `GET /metrics` request line.
fn is_metrics_request(request: &str) -> bool {
    let line = request.lines().next().unwrap_or("");
    let mut parts = line.split_whitespace();
    parts.next() == Some("GET") && matches!(parts.next(), Some("/metrics") | Some("/metrics/"))
}

/// Serves `/metrics` synchronously, accepting connections until the process
/// ends. Intended to run on its own thread; one scrape per connection.
///
/// The closure is called per request to obtain a fresh snapshot of the
/// registry, so metrics are current at scrape time.
pub fn serve(
    addr: impl ToSocketAddrs,
    mut snapshot: impl FnMut() -> MetricsRegistry,
) -> std::io::Result<()> {
    let listener = TcpListener::bind(addr)?;
    for stream in listener.incoming() {
        let mut stream = match stream {
            Ok(s) => s,
            Err(_) => continue,
        };
        let mut buf = [0u8; 1024];
        let n = stream.read(&mut buf).unwrap_or(0);
        let request = String::from_utf8_lossy(&buf[..n]);
        let response = if is_metrics_request(&request) {
            http_response(&snapshot())
        } else {
            not_found_response()
        };
        let _ = stream.write_all(response.as_bytes());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::telemetry::Subsystem;

    #[test]
    fn counters_and_gauges_aggregate() {
        let mut reg = MetricsRegistry::new();
        reg.ingest(&Signal::Counter {
            name: "frames",
            subsystem: Subsystem::Render,
            increment: 10,
        });
        reg.ingest(&Signal::Counter {
            name: "frames",
            subsystem: Subsystem::Render,
            increment: 5,
        });
        reg.ingest(&Signal::Gauge {
            name: "fps",
            subsystem: Subsystem::Render,
            value: 59.9,
            unit: "hz",
        });
        assert_eq!(reg.counter("frames"), Some(15));
        assert_eq!(reg.gauge("fps"), Some(59.9));
    }

    #[test]
    fn render_is_prometheus_text_and_deterministic() {
        let mut reg = MetricsRegistry::new();
        reg.ingest(&Signal::Counter {
            name: "draws",
            subsystem: Subsystem::Render,
            increment: 3,
        });
        let text = reg.render();
        assert!(text.contains("# TYPE draws counter\n"));
        assert!(text.contains("draws 3\n"));
        assert_eq!(text, reg.render()); // stable
    }

    #[test]
    fn invalid_metric_name_characters_are_sanitized() {
        assert_eq!(metric_name("ai.path.find"), "ai_path_find");
        assert_eq!(metric_name("3frames"), "_3frames");
        assert_eq!(metric_name("ok_name:1"), "ok_name:1");
    }

    #[test]
    fn http_response_carries_the_body_and_length() {
        let mut reg = MetricsRegistry::new();
        reg.ingest(&Signal::Counter {
            name: "x",
            subsystem: Subsystem::Telemetry,
            increment: 1,
        });
        let response = http_response(&reg);
        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
        let body = reg.render();
        assert!(response.ends_with(&body));
        assert!(response.contains(&format!("Content-Length: {}", body.len())));
    }

    #[test]
    fn only_the_metrics_route_is_recognized() {
        assert!(is_metrics_request("GET /metrics HTTP/1.1\r\n"));
        assert!(is_metrics_request("GET /metrics/ HTTP/1.1\r\n"));
        assert!(!is_metrics_request("GET / HTTP/1.1\r\n"));
        assert!(!is_metrics_request("POST /metrics HTTP/1.1\r\n"));
    }
}
