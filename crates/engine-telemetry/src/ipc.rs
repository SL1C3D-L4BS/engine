//! Telemetry IPC: the wire protocol external tools speak to the engine.
//!
//! A debugger, profiler, or the editor connects to the engine over a
//! Unix-domain socket and exchanges [`Message`]s. The framing is
//! length-prefixed — `[u32 little-endian body length][body]` — and the body
//! is an owned compact binary encoding (spec X.5).
//!
//! The body encoding is owned rather than delegated to MessagePack: the
//! message set is small, fixed, and versioned with the engine, so owning it
//! removes a dependency and a class of schema-drift bugs (R-02).

use engine_core::telemetry::{Signal, Subsystem};

/// The socket path for the telemetry IPC endpoint of a given process.
pub fn socket_path(pid: u32) -> String {
    format!("/tmp/engine-{pid}.sock")
}

/// A signal in owned form — the on-ring [`Signal`] borrows `&'static str`
/// names, which cannot survive a decode, so the wire form owns its strings.
#[derive(Clone, Debug, PartialEq)]
pub enum WireSignal {
    /// A timed operation. See [`Signal::Span`].
    Span {
        /// Operation name.
        name: String,
        /// Originating subsystem tag.
        subsystem: u8,
        /// Start timestamp, nanoseconds.
        start_ns: u64,
        /// End timestamp, nanoseconds.
        end_ns: u64,
    },
    /// A counter increment. See [`Signal::Counter`].
    Counter {
        /// Counter name.
        name: String,
        /// Originating subsystem tag.
        subsystem: u8,
        /// Amount added.
        increment: u64,
    },
    /// A gauge sample. See [`Signal::Gauge`].
    Gauge {
        /// Gauge name.
        name: String,
        /// Originating subsystem tag.
        subsystem: u8,
        /// Measured value.
        value: f64,
        /// Unit label.
        unit: String,
    },
    /// A discrete event. See [`Signal::Event`].
    Event {
        /// Event name.
        name: String,
        /// Originating subsystem tag.
        subsystem: u8,
        /// Flat key/value payload.
        fields: Vec<(String, String)>,
    },
}

impl WireSignal {
    /// Converts a borrowed ring [`Signal`] into its owned wire form.
    pub fn from_signal(signal: &Signal) -> Self {
        match signal {
            Signal::Span {
                name,
                subsystem,
                start_ns,
                end_ns,
            } => Self::Span {
                name: (*name).to_string(),
                subsystem: subsystem.as_u8(),
                start_ns: *start_ns,
                end_ns: *end_ns,
            },
            Signal::Counter {
                name,
                subsystem,
                increment,
            } => Self::Counter {
                name: (*name).to_string(),
                subsystem: subsystem.as_u8(),
                increment: *increment,
            },
            Signal::Gauge {
                name,
                subsystem,
                value,
                unit,
            } => Self::Gauge {
                name: (*name).to_string(),
                subsystem: subsystem.as_u8(),
                value: *value,
                unit: (*unit).to_string(),
            },
            Signal::Event {
                name,
                subsystem,
                fields,
            } => Self::Event {
                name: (*name).to_string(),
                subsystem: subsystem.as_u8(),
                fields: fields.clone(),
            },
        }
    }
}

/// Maps a subsystem tag back to its [`Subsystem`], if it is in range.
pub fn subsystem_from_u8(tag: u8) -> Option<Subsystem> {
    use Subsystem::*;
    Some(match tag {
        0 => Ecs,
        1 => Render,
        2 => Physics,
        3 => Audio,
        4 => Net,
        5 => Script,
        6 => Ai,
        7 => Asset,
        8 => Editor,
        9 => Hub,
        10 => Platform,
        11 => Telemetry,
        _ => return None,
    })
}

/// A telemetry IPC message (spec X.5 message types `0x01`–`0x0A`).
#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    /// `0x01` — client greeting: protocol version and the client's PID.
    Hello {
        /// Protocol version the client speaks.
        version: u32,
        /// Client process id.
        pid: u32,
    },
    /// `0x02` — server acknowledgement of a [`Message::Hello`].
    Welcome {
        /// Protocol version the server speaks.
        version: u32,
    },
    /// `0x03` — subscribe to a subsystem bitmask (`1 << Subsystem::as_u8()`).
    Subscribe {
        /// Bitmask of subsystems to receive.
        mask: u32,
    },
    /// `0x04` — cancel a prior [`Message::Subscribe`].
    Unsubscribe,
    /// `0x05` — a batch of telemetry signals.
    Signals(Vec<WireSignal>),
    /// `0x06` — a frame boundary marker.
    FrameMark {
        /// Monotonic frame index.
        frame: u64,
    },
    /// `0x07` — a structured log line (already-rendered JSON).
    LogLine(String),
    /// `0x08` — the count of signals dropped to ring overflow.
    DropReport {
        /// Number of dropped signals.
        dropped: u64,
    },
    /// `0x09` — an error notice.
    Error(String),
    /// `0x0A` — orderly disconnect.
    Goodbye,
}

/// A failure decoding a [`Message`] frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IpcError {
    /// The buffer ended before the message did.
    Truncated,
    /// The message type byte is not one this build knows.
    UnknownType(u8),
    /// A string field was not valid UTF-8.
    BadUtf8,
    /// A signal or subsystem tag was out of range.
    BadTag,
}

const PROTOCOL_VERSION: u32 = 1;

/// The protocol version this build implements.
pub fn protocol_version() -> u32 {
    PROTOCOL_VERSION
}

/// Encodes `message` into a complete length-prefixed frame.
pub fn encode(message: &Message) -> Vec<u8> {
    let mut body = Vec::new();
    encode_body(message, &mut body);
    let mut frame = Vec::with_capacity(4 + body.len());
    frame.extend_from_slice(&(body.len() as u32).to_le_bytes());
    frame.extend_from_slice(&body);
    frame
}

/// Decodes one length-prefixed frame, returning the message and the number of
/// bytes consumed (so a stream can be decoded frame by frame).
pub fn decode(buf: &[u8]) -> Result<(Message, usize), IpcError> {
    let len = u32::from_le_bytes(
        buf.get(0..4)
            .ok_or(IpcError::Truncated)?
            .try_into()
            .unwrap(),
    ) as usize;
    let body = buf.get(4..4 + len).ok_or(IpcError::Truncated)?;
    let mut r = Reader::new(body);
    let message = decode_body(&mut r)?;
    Ok((message, 4 + len))
}

fn encode_body(message: &Message, out: &mut Vec<u8>) {
    match message {
        Message::Hello { version, pid } => {
            out.push(0x01);
            out.extend_from_slice(&version.to_le_bytes());
            out.extend_from_slice(&pid.to_le_bytes());
        }
        Message::Welcome { version } => {
            out.push(0x02);
            out.extend_from_slice(&version.to_le_bytes());
        }
        Message::Subscribe { mask } => {
            out.push(0x03);
            out.extend_from_slice(&mask.to_le_bytes());
        }
        Message::Unsubscribe => out.push(0x04),
        Message::Signals(signals) => {
            out.push(0x05);
            out.extend_from_slice(&(signals.len() as u32).to_le_bytes());
            for signal in signals {
                encode_signal(signal, out);
            }
        }
        Message::FrameMark { frame } => {
            out.push(0x06);
            out.extend_from_slice(&frame.to_le_bytes());
        }
        Message::LogLine(line) => {
            out.push(0x07);
            encode_str(line, out);
        }
        Message::DropReport { dropped } => {
            out.push(0x08);
            out.extend_from_slice(&dropped.to_le_bytes());
        }
        Message::Error(text) => {
            out.push(0x09);
            encode_str(text, out);
        }
        Message::Goodbye => out.push(0x0A),
    }
}

fn decode_body(r: &mut Reader<'_>) -> Result<Message, IpcError> {
    Ok(match r.u8()? {
        0x01 => Message::Hello {
            version: r.u32()?,
            pid: r.u32()?,
        },
        0x02 => Message::Welcome { version: r.u32()? },
        0x03 => Message::Subscribe { mask: r.u32()? },
        0x04 => Message::Unsubscribe,
        0x05 => {
            let count = r.u32()? as usize;
            let mut signals = Vec::with_capacity(count);
            for _ in 0..count {
                signals.push(decode_signal(r)?);
            }
            Message::Signals(signals)
        }
        0x06 => Message::FrameMark { frame: r.u64()? },
        0x07 => Message::LogLine(r.string()?),
        0x08 => Message::DropReport { dropped: r.u64()? },
        0x09 => Message::Error(r.string()?),
        0x0A => Message::Goodbye,
        other => return Err(IpcError::UnknownType(other)),
    })
}

fn encode_signal(signal: &WireSignal, out: &mut Vec<u8>) {
    match signal {
        WireSignal::Span {
            name,
            subsystem,
            start_ns,
            end_ns,
        } => {
            out.push(0);
            encode_str(name, out);
            out.push(*subsystem);
            out.extend_from_slice(&start_ns.to_le_bytes());
            out.extend_from_slice(&end_ns.to_le_bytes());
        }
        WireSignal::Counter {
            name,
            subsystem,
            increment,
        } => {
            out.push(1);
            encode_str(name, out);
            out.push(*subsystem);
            out.extend_from_slice(&increment.to_le_bytes());
        }
        WireSignal::Gauge {
            name,
            subsystem,
            value,
            unit,
        } => {
            out.push(2);
            encode_str(name, out);
            out.push(*subsystem);
            out.extend_from_slice(&value.to_le_bytes());
            encode_str(unit, out);
        }
        WireSignal::Event {
            name,
            subsystem,
            fields,
        } => {
            out.push(3);
            encode_str(name, out);
            out.push(*subsystem);
            out.extend_from_slice(&(fields.len() as u32).to_le_bytes());
            for (key, value) in fields {
                encode_str(key, out);
                encode_str(value, out);
            }
        }
    }
}

fn decode_signal(r: &mut Reader<'_>) -> Result<WireSignal, IpcError> {
    Ok(match r.u8()? {
        0 => WireSignal::Span {
            name: r.string()?,
            subsystem: check_subsystem(r.u8()?)?,
            start_ns: r.u64()?,
            end_ns: r.u64()?,
        },
        1 => WireSignal::Counter {
            name: r.string()?,
            subsystem: check_subsystem(r.u8()?)?,
            increment: r.u64()?,
        },
        2 => WireSignal::Gauge {
            name: r.string()?,
            subsystem: check_subsystem(r.u8()?)?,
            value: r.f64()?,
            unit: r.string()?,
        },
        3 => {
            let name = r.string()?;
            let subsystem = check_subsystem(r.u8()?)?;
            let count = r.u32()? as usize;
            let mut fields = Vec::with_capacity(count);
            for _ in 0..count {
                fields.push((r.string()?, r.string()?));
            }
            WireSignal::Event {
                name,
                subsystem,
                fields,
            }
        }
        _ => return Err(IpcError::BadTag),
    })
}

fn check_subsystem(tag: u8) -> Result<u8, IpcError> {
    subsystem_from_u8(tag).map(|_| tag).ok_or(IpcError::BadTag)
}

fn encode_str(s: &str, out: &mut Vec<u8>) {
    out.extend_from_slice(&(s.len() as u32).to_le_bytes());
    out.extend_from_slice(s.as_bytes());
}

/// A forward cursor over a frame body.
struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], IpcError> {
        let end = self.pos.checked_add(n).ok_or(IpcError::Truncated)?;
        let slice = self.data.get(self.pos..end).ok_or(IpcError::Truncated)?;
        self.pos = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, IpcError> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, IpcError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn u64(&mut self) -> Result<u64, IpcError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn f64(&mut self) -> Result<f64, IpcError> {
        Ok(f64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn string(&mut self) -> Result<String, IpcError> {
        let len = self.u32()? as usize;
        let bytes = self.take(len)?;
        std::str::from_utf8(bytes)
            .map(str::to_string)
            .map_err(|_| IpcError::BadUtf8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(message: Message) {
        let frame = encode(&message);
        let (decoded, consumed) = decode(&frame).expect("decodes");
        assert_eq!(decoded, message);
        assert_eq!(consumed, frame.len());
    }

    #[test]
    fn every_message_type_round_trips() {
        round_trip(Message::Hello {
            version: protocol_version(),
            pid: 4321,
        });
        round_trip(Message::Welcome { version: 1 });
        round_trip(Message::Subscribe { mask: 0b1011 });
        round_trip(Message::Unsubscribe);
        round_trip(Message::Signals(vec![
            WireSignal::Span {
                name: "frame".into(),
                subsystem: Subsystem::Render.as_u8(),
                start_ns: 10,
                end_ns: 99,
            },
            WireSignal::Counter {
                name: "draws".into(),
                subsystem: Subsystem::Render.as_u8(),
                increment: 7,
            },
            WireSignal::Gauge {
                name: "vram".into(),
                subsystem: Subsystem::Render.as_u8(),
                value: 1536.5,
                unit: "MiB".into(),
            },
            WireSignal::Event {
                name: "reload".into(),
                subsystem: Subsystem::Asset.as_u8(),
                fields: vec![("path".into(), "a.tex".into())],
            },
        ]));
        round_trip(Message::FrameMark { frame: 9_001 });
        round_trip(Message::LogLine("{\"msg\":\"hi\"}".into()));
        round_trip(Message::DropReport { dropped: 42 });
        round_trip(Message::Error("connection lost".into()));
        round_trip(Message::Goodbye);
    }

    #[test]
    fn truncated_frame_is_rejected() {
        let frame = encode(&Message::FrameMark { frame: 1 });
        assert_eq!(decode(&frame[..frame.len() - 1]), Err(IpcError::Truncated));
        assert_eq!(decode(&[0, 0]), Err(IpcError::Truncated));
    }

    #[test]
    fn unknown_type_is_rejected() {
        // Body length 1, body is a single unknown type byte.
        let frame = [1, 0, 0, 0, 0x7F];
        assert_eq!(decode(&frame), Err(IpcError::UnknownType(0x7F)));
    }

    #[test]
    fn signal_conversion_preserves_fields() {
        let signal = Signal::Counter {
            name: "ticks",
            subsystem: Subsystem::Ecs,
            increment: 3,
        };
        assert_eq!(
            WireSignal::from_signal(&signal),
            WireSignal::Counter {
                name: "ticks".into(),
                subsystem: 0,
                increment: 3,
            }
        );
    }

    #[test]
    fn socket_path_is_pid_scoped() {
        assert_eq!(socket_path(7), "/tmp/engine-7.sock");
    }
}
