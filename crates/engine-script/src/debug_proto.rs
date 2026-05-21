//! Owned binary debugger protocol (spec XII, ADR-036).
//!
//! NO Microsoft DAP JSON; we own the wire. Each frame is
//! `[u32 LE body length][u16 LE proto_version][u8 kind][body]`.
//! `proto_version = 0x0001` ships in v0.4. Reuses the
//! `engine_telemetry::ipc` framing envelope (`[u32 LE len][body]`)
//! over the same UDS — the connection handshake byte selects the
//! debugger protocol vs the telemetry stream.
//!
//! Phase 4 PR 3 ships every request/event from the spec-XII
//! affordance table so the Phase 10 editor work is UI-only. The
//! `examples/editor-bridge.rs` reference client exercises the full
//! surface.

use crate::debug::BreakpointId;

/// Protocol version this build implements.
pub const PROTOCOL_VERSION: u16 = 0x0001;

/// One scalar value the debugger ships over the wire. Lossy
/// projection of `crate::vm::Value` — aggregates render via a short
/// summary string so the wire stays compact.
#[derive(Clone, Debug, PartialEq)]
pub enum WireValue {
    /// `nil`
    Nil,
    /// Boolean.
    Bool(bool),
    /// Signed 64-bit integer.
    Int(i64),
    /// 64-bit float (bit pattern for total equality).
    FloatBits(u64),
    /// UTF-8 string.
    Str(String),
    /// Opaque aggregate summary (e.g. `Array(3 items)`).
    Summary(String),
}

/// Description of one stack frame in a `ListFrames` response.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameInfo {
    /// Frame index (0 = top of stack).
    pub index: u32,
    /// Function name.
    pub function: String,
    /// 1-based source line at the current PC.
    pub line: u32,
}

/// One register in a `ListLocals` response.
#[derive(Clone, Debug, PartialEq)]
pub struct Local {
    /// Local name (`r0`, `r1`, … in PR 3 — codegen has no local
    /// name table yet).
    pub name: String,
    /// Register index.
    pub reg: u8,
    /// Current value.
    pub value: WireValue,
    /// Whether the value changed since the last pause.
    pub dirty: bool,
}

/// Client → server requests.
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    /// Install a breakpoint at `(file_id, line)`.
    SetBreakpoint {
        /// Source file id.
        file_id: u32,
        /// 1-based line number.
        line: u32,
        /// Optional condition expression.
        condition: Option<String>,
        /// Optional hit-count gate.
        hit_count: Option<u32>,
    },
    /// Install a function-name breakpoint.
    SetFunctionBreakpoint {
        /// Function name.
        fn_name: String,
    },
    /// Subscribe to exception breakpoints.
    SetExceptionBreakpoint {
        /// Bitmask of exception classes.
        mask: u32,
    },
    /// Remove a breakpoint.
    ClearBreakpoint {
        /// Id returned by a prior `SetBreakpoint`.
        id: BreakpointId,
    },
    /// Continue execution.
    Continue,
    /// Pause execution at the next safepoint.
    Pause,
    /// Step over the next source line.
    StepOver,
    /// Step into the next call.
    StepInto,
    /// Step out of the current frame.
    StepOut,
    /// Continue until execution reaches `(file_id, line)`.
    RunToCursor {
        /// Target file id.
        file_id: u32,
        /// Target line.
        line: u32,
    },
    /// Request the current call stack.
    ListFrames,
    /// Request the locals of a frame.
    ListLocals {
        /// Frame index, 0 = top.
        frame_id: u32,
    },
    /// Expand a structured value at `path`.
    ExpandValue {
        /// Frame the path is relative to.
        frame_id: u32,
        /// Dot-separated path (e.g. `r2.x`).
        path: String,
    },
    /// Mutate a local.
    SetLocal {
        /// Frame index.
        frame_id: u32,
        /// Local name.
        name: String,
        /// New value (debugger-side serialisation).
        value: WireValue,
    },
    /// Install a watch expression.
    Watch {
        /// Source of the expression.
        expr: String,
    },
    /// Remove a watch expression.
    Unwatch {
        /// Id returned by a prior `Watch`.
        id: u32,
    },
    /// Evaluate a const expression against the loaded module.
    EvalConst {
        /// Source of the expression.
        expr: String,
    },
    /// Disconnect the debugger client.
    Detach,
}

/// Server → client responses to a request. The wire pairs a `Request`
/// with one `Response`.
#[derive(Clone, Debug, PartialEq)]
pub enum Response {
    /// Generic ack with no payload.
    Ack,
    /// Allocated breakpoint id.
    BreakpointId(BreakpointId),
    /// Allocated watch id.
    WatchId(u32),
    /// Resolved const value.
    ConstValue(WireValue),
    /// Snapshot of the call stack.
    Frames(Vec<FrameInfo>),
    /// Snapshot of one frame's locals.
    Locals(Vec<Local>),
    /// Expanded value.
    Value(WireValue),
    /// The request failed; carries an error string.
    Error(String),
}

/// Server → client unsolicited events (no matching `Request`).
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// Execution paused.
    Stopped {
        /// Why the VM paused.
        reason: StopReason,
        /// Fiber id (always `0` in PR 3).
        fiber_id: u32,
        /// Top frame index.
        frame_id: u32,
    },
    /// A breakpoint was hit.
    BreakpointHit {
        /// Id of the breakpoint.
        id: BreakpointId,
        /// Fiber id.
        fiber_id: u32,
    },
    /// An exception fired.
    Exception {
        /// Fiber id.
        fiber_id: u32,
        /// Interned message id.
        message_id: u32,
    },
    /// One line of script output.
    OutputLine {
        /// `0` = stdout, `1` = stderr.
        stream: u8,
        /// Text content.
        text: String,
    },
    /// A module was hot-reloaded; breakpoints have been re-armed.
    ModuleReloaded {
        /// File id of the reloaded source.
        file_id: u32,
    },
    /// Watch expression updated.
    WatchUpdate {
        /// Watch id.
        id: u32,
        /// New value.
        value: WireValue,
    },
}

/// Why execution stopped, carried by [`Event::Stopped`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    /// `Pause` request.
    Pause,
    /// `StepOver`, `StepInto`, `StepOut` completed.
    Step,
    /// A breakpoint fired.
    Breakpoint,
    /// An exception fired.
    Exception,
    /// The VM entered an FFI sink that asked the debugger to pause.
    FfiPause,
    /// A `RunToCursor` reached its target.
    Cursor,
}

// --- wire encoding ----------------------------------------------------------

/// Encodes a request into a length-prefixed framed payload.
pub fn encode_request(req: &Request) -> Vec<u8> {
    let mut body = vec![tag_for_request(req)];
    encode_request_body(req, &mut body);
    frame(&body)
}

/// Encodes a response into a framed payload.
pub fn encode_response(resp: &Response) -> Vec<u8> {
    let mut body = vec![tag_for_response(resp)];
    encode_response_body(resp, &mut body);
    frame(&body)
}

/// Encodes an event into a framed payload.
pub fn encode_event(ev: &Event) -> Vec<u8> {
    let mut body = vec![tag_for_event(ev)];
    encode_event_body(ev, &mut body);
    frame(&body)
}

fn frame(body: &[u8]) -> Vec<u8> {
    let len = (body.len() + 2) as u32; // +2 for proto_version
    let mut out = Vec::with_capacity(4 + 2 + body.len());
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
    out.extend_from_slice(body);
    out
}

/// Decodes one framed message, returning the parsed body and the
/// number of bytes consumed.
pub fn decode_frame(bytes: &[u8]) -> Result<(u16, Vec<u8>, usize), ProtoError> {
    if bytes.len() < 4 {
        return Err(ProtoError::Truncated);
    }
    let len = u32::from_le_bytes(bytes[0..4].try_into().unwrap()) as usize;
    if bytes.len() < 4 + len {
        return Err(ProtoError::Truncated);
    }
    if len < 2 {
        return Err(ProtoError::BadVersion);
    }
    let version = u16::from_le_bytes(bytes[4..6].try_into().unwrap());
    let body = bytes[6..4 + len].to_vec();
    Ok((version, body, 4 + len))
}

/// Decodes a request body (everything after the proto-version bytes).
pub fn decode_request(body: &[u8]) -> Result<Request, ProtoError> {
    let mut r = ByteReader::new(body);
    let tag = r.u8()?;
    Ok(match tag {
        0x01 => Request::SetBreakpoint {
            file_id: r.u32()?,
            line: r.u32()?,
            condition: r.opt_str()?,
            hit_count: r.opt_u32()?,
        },
        0x02 => Request::SetFunctionBreakpoint { fn_name: r.str()? },
        0x03 => Request::SetExceptionBreakpoint { mask: r.u32()? },
        0x04 => Request::ClearBreakpoint {
            id: BreakpointId(r.u32()?),
        },
        0x05 => Request::Continue,
        0x06 => Request::Pause,
        0x07 => Request::StepOver,
        0x08 => Request::StepInto,
        0x09 => Request::StepOut,
        0x0A => Request::RunToCursor {
            file_id: r.u32()?,
            line: r.u32()?,
        },
        0x0B => Request::ListFrames,
        0x0C => Request::ListLocals { frame_id: r.u32()? },
        0x0D => Request::ExpandValue {
            frame_id: r.u32()?,
            path: r.str()?,
        },
        0x0E => Request::SetLocal {
            frame_id: r.u32()?,
            name: r.str()?,
            value: r.value()?,
        },
        0x0F => Request::Watch { expr: r.str()? },
        0x10 => Request::Unwatch { id: r.u32()? },
        0x11 => Request::EvalConst { expr: r.str()? },
        0x12 => Request::Detach,
        other => return Err(ProtoError::UnknownTag(other)),
    })
}

/// Decodes a response body.
pub fn decode_response(body: &[u8]) -> Result<Response, ProtoError> {
    let mut r = ByteReader::new(body);
    let tag = r.u8()?;
    Ok(match tag {
        0x00 => Response::Ack,
        0x01 => Response::BreakpointId(BreakpointId(r.u32()?)),
        0x02 => Response::WatchId(r.u32()?),
        0x03 => Response::ConstValue(r.value()?),
        0x04 => {
            let n = r.u32()? as usize;
            let mut frames = Vec::with_capacity(n);
            for _ in 0..n {
                frames.push(FrameInfo {
                    index: r.u32()?,
                    function: r.str()?,
                    line: r.u32()?,
                });
            }
            Response::Frames(frames)
        }
        0x05 => {
            let n = r.u32()? as usize;
            let mut locals = Vec::with_capacity(n);
            for _ in 0..n {
                locals.push(Local {
                    name: r.str()?,
                    reg: r.u8()?,
                    value: r.value()?,
                    dirty: r.u8()? != 0,
                });
            }
            Response::Locals(locals)
        }
        0x06 => Response::Value(r.value()?),
        0x07 => Response::Error(r.str()?),
        other => return Err(ProtoError::UnknownTag(other)),
    })
}

/// Decodes an event body.
pub fn decode_event(body: &[u8]) -> Result<Event, ProtoError> {
    let mut r = ByteReader::new(body);
    let tag = r.u8()?;
    Ok(match tag {
        0x01 => Event::Stopped {
            reason: stop_reason_from_u8(r.u8()?)?,
            fiber_id: r.u32()?,
            frame_id: r.u32()?,
        },
        0x02 => Event::BreakpointHit {
            id: BreakpointId(r.u32()?),
            fiber_id: r.u32()?,
        },
        0x03 => Event::Exception {
            fiber_id: r.u32()?,
            message_id: r.u32()?,
        },
        0x04 => Event::OutputLine {
            stream: r.u8()?,
            text: r.str()?,
        },
        0x05 => Event::ModuleReloaded { file_id: r.u32()? },
        0x06 => Event::WatchUpdate {
            id: r.u32()?,
            value: r.value()?,
        },
        other => return Err(ProtoError::UnknownTag(other)),
    })
}

// --- private encoders -------------------------------------------------------

fn tag_for_request(r: &Request) -> u8 {
    match r {
        Request::SetBreakpoint { .. } => 0x01,
        Request::SetFunctionBreakpoint { .. } => 0x02,
        Request::SetExceptionBreakpoint { .. } => 0x03,
        Request::ClearBreakpoint { .. } => 0x04,
        Request::Continue => 0x05,
        Request::Pause => 0x06,
        Request::StepOver => 0x07,
        Request::StepInto => 0x08,
        Request::StepOut => 0x09,
        Request::RunToCursor { .. } => 0x0A,
        Request::ListFrames => 0x0B,
        Request::ListLocals { .. } => 0x0C,
        Request::ExpandValue { .. } => 0x0D,
        Request::SetLocal { .. } => 0x0E,
        Request::Watch { .. } => 0x0F,
        Request::Unwatch { .. } => 0x10,
        Request::EvalConst { .. } => 0x11,
        Request::Detach => 0x12,
    }
}

fn tag_for_response(r: &Response) -> u8 {
    match r {
        Response::Ack => 0x00,
        Response::BreakpointId(_) => 0x01,
        Response::WatchId(_) => 0x02,
        Response::ConstValue(_) => 0x03,
        Response::Frames(_) => 0x04,
        Response::Locals(_) => 0x05,
        Response::Value(_) => 0x06,
        Response::Error(_) => 0x07,
    }
}

fn tag_for_event(e: &Event) -> u8 {
    match e {
        Event::Stopped { .. } => 0x01,
        Event::BreakpointHit { .. } => 0x02,
        Event::Exception { .. } => 0x03,
        Event::OutputLine { .. } => 0x04,
        Event::ModuleReloaded { .. } => 0x05,
        Event::WatchUpdate { .. } => 0x06,
    }
}

fn encode_request_body(req: &Request, out: &mut Vec<u8>) {
    match req {
        Request::SetBreakpoint {
            file_id,
            line,
            condition,
            hit_count,
        } => {
            out.extend_from_slice(&file_id.to_le_bytes());
            out.extend_from_slice(&line.to_le_bytes());
            write_opt_str(condition.as_deref(), out);
            write_opt_u32(*hit_count, out);
        }
        Request::SetFunctionBreakpoint { fn_name } => write_str(fn_name, out),
        Request::SetExceptionBreakpoint { mask } => out.extend_from_slice(&mask.to_le_bytes()),
        Request::ClearBreakpoint { id } => out.extend_from_slice(&id.0.to_le_bytes()),
        Request::Continue
        | Request::Pause
        | Request::StepOver
        | Request::StepInto
        | Request::StepOut
        | Request::ListFrames
        | Request::Detach => {}
        Request::RunToCursor { file_id, line } => {
            out.extend_from_slice(&file_id.to_le_bytes());
            out.extend_from_slice(&line.to_le_bytes());
        }
        Request::ListLocals { frame_id } => out.extend_from_slice(&frame_id.to_le_bytes()),
        Request::ExpandValue { frame_id, path } => {
            out.extend_from_slice(&frame_id.to_le_bytes());
            write_str(path, out);
        }
        Request::SetLocal {
            frame_id,
            name,
            value,
        } => {
            out.extend_from_slice(&frame_id.to_le_bytes());
            write_str(name, out);
            write_value(value, out);
        }
        Request::Watch { expr } => write_str(expr, out),
        Request::Unwatch { id } => out.extend_from_slice(&id.to_le_bytes()),
        Request::EvalConst { expr } => write_str(expr, out),
    }
}

fn encode_response_body(resp: &Response, out: &mut Vec<u8>) {
    match resp {
        Response::Ack => {}
        Response::BreakpointId(id) => out.extend_from_slice(&id.0.to_le_bytes()),
        Response::WatchId(id) => out.extend_from_slice(&id.to_le_bytes()),
        Response::ConstValue(v) => write_value(v, out),
        Response::Frames(frames) => {
            out.extend_from_slice(&(frames.len() as u32).to_le_bytes());
            for f in frames {
                out.extend_from_slice(&f.index.to_le_bytes());
                write_str(&f.function, out);
                out.extend_from_slice(&f.line.to_le_bytes());
            }
        }
        Response::Locals(locals) => {
            out.extend_from_slice(&(locals.len() as u32).to_le_bytes());
            for l in locals {
                write_str(&l.name, out);
                out.push(l.reg);
                write_value(&l.value, out);
                out.push(if l.dirty { 1 } else { 0 });
            }
        }
        Response::Value(v) => write_value(v, out),
        Response::Error(s) => write_str(s, out),
    }
}

fn encode_event_body(ev: &Event, out: &mut Vec<u8>) {
    match ev {
        Event::Stopped {
            reason,
            fiber_id,
            frame_id,
        } => {
            out.push(stop_reason_to_u8(*reason));
            out.extend_from_slice(&fiber_id.to_le_bytes());
            out.extend_from_slice(&frame_id.to_le_bytes());
        }
        Event::BreakpointHit { id, fiber_id } => {
            out.extend_from_slice(&id.0.to_le_bytes());
            out.extend_from_slice(&fiber_id.to_le_bytes());
        }
        Event::Exception {
            fiber_id,
            message_id,
        } => {
            out.extend_from_slice(&fiber_id.to_le_bytes());
            out.extend_from_slice(&message_id.to_le_bytes());
        }
        Event::OutputLine { stream, text } => {
            out.push(*stream);
            write_str(text, out);
        }
        Event::ModuleReloaded { file_id } => out.extend_from_slice(&file_id.to_le_bytes()),
        Event::WatchUpdate { id, value } => {
            out.extend_from_slice(&id.to_le_bytes());
            write_value(value, out);
        }
    }
}

fn stop_reason_to_u8(r: StopReason) -> u8 {
    match r {
        StopReason::Pause => 0x01,
        StopReason::Step => 0x02,
        StopReason::Breakpoint => 0x03,
        StopReason::Exception => 0x04,
        StopReason::FfiPause => 0x05,
        StopReason::Cursor => 0x06,
    }
}

fn stop_reason_from_u8(b: u8) -> Result<StopReason, ProtoError> {
    Ok(match b {
        0x01 => StopReason::Pause,
        0x02 => StopReason::Step,
        0x03 => StopReason::Breakpoint,
        0x04 => StopReason::Exception,
        0x05 => StopReason::FfiPause,
        0x06 => StopReason::Cursor,
        other => return Err(ProtoError::UnknownTag(other)),
    })
}

fn write_str(s: &str, out: &mut Vec<u8>) {
    out.extend_from_slice(&(s.len() as u32).to_le_bytes());
    out.extend_from_slice(s.as_bytes());
}

fn write_opt_str(s: Option<&str>, out: &mut Vec<u8>) {
    match s {
        Some(s) => {
            out.push(1);
            write_str(s, out);
        }
        None => out.push(0),
    }
}

fn write_opt_u32(v: Option<u32>, out: &mut Vec<u8>) {
    match v {
        Some(v) => {
            out.push(1);
            out.extend_from_slice(&v.to_le_bytes());
        }
        None => out.push(0),
    }
}

fn write_value(v: &WireValue, out: &mut Vec<u8>) {
    match v {
        WireValue::Nil => out.push(0),
        WireValue::Bool(b) => {
            out.push(1);
            out.push(if *b { 1 } else { 0 });
        }
        WireValue::Int(i) => {
            out.push(2);
            out.extend_from_slice(&i.to_le_bytes());
        }
        WireValue::FloatBits(b) => {
            out.push(3);
            out.extend_from_slice(&b.to_le_bytes());
        }
        WireValue::Str(s) => {
            out.push(4);
            write_str(s, out);
        }
        WireValue::Summary(s) => {
            out.push(5);
            write_str(s, out);
        }
    }
}

struct ByteReader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> ByteReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn u8(&mut self) -> Result<u8, ProtoError> {
        let b = *self.bytes.get(self.pos).ok_or(ProtoError::Truncated)?;
        self.pos += 1;
        Ok(b)
    }

    fn u32(&mut self) -> Result<u32, ProtoError> {
        let end = self.pos + 4;
        if end > self.bytes.len() {
            return Err(ProtoError::Truncated);
        }
        let v = u32::from_le_bytes(self.bytes[self.pos..end].try_into().unwrap());
        self.pos = end;
        Ok(v)
    }

    fn i64(&mut self) -> Result<i64, ProtoError> {
        let end = self.pos + 8;
        if end > self.bytes.len() {
            return Err(ProtoError::Truncated);
        }
        let v = i64::from_le_bytes(self.bytes[self.pos..end].try_into().unwrap());
        self.pos = end;
        Ok(v)
    }

    fn u64(&mut self) -> Result<u64, ProtoError> {
        let end = self.pos + 8;
        if end > self.bytes.len() {
            return Err(ProtoError::Truncated);
        }
        let v = u64::from_le_bytes(self.bytes[self.pos..end].try_into().unwrap());
        self.pos = end;
        Ok(v)
    }

    fn str(&mut self) -> Result<String, ProtoError> {
        let len = self.u32()? as usize;
        let end = self.pos + len;
        if end > self.bytes.len() {
            return Err(ProtoError::Truncated);
        }
        let s = String::from_utf8(self.bytes[self.pos..end].to_vec())
            .map_err(|_| ProtoError::BadUtf8)?;
        self.pos = end;
        Ok(s)
    }

    fn opt_str(&mut self) -> Result<Option<String>, ProtoError> {
        match self.u8()? {
            0 => Ok(None),
            1 => Ok(Some(self.str()?)),
            _ => Err(ProtoError::BadOptional),
        }
    }

    fn opt_u32(&mut self) -> Result<Option<u32>, ProtoError> {
        match self.u8()? {
            0 => Ok(None),
            1 => Ok(Some(self.u32()?)),
            _ => Err(ProtoError::BadOptional),
        }
    }

    fn value(&mut self) -> Result<WireValue, ProtoError> {
        Ok(match self.u8()? {
            0 => WireValue::Nil,
            1 => WireValue::Bool(self.u8()? != 0),
            2 => WireValue::Int(self.i64()?),
            3 => WireValue::FloatBits(self.u64()?),
            4 => WireValue::Str(self.str()?),
            5 => WireValue::Summary(self.str()?),
            other => return Err(ProtoError::UnknownTag(other)),
        })
    }
}

/// Why a frame could not be decoded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProtoError {
    /// The buffer ended before the message did.
    Truncated,
    /// The proto-version field was missing.
    BadVersion,
    /// An optional-field tag was neither `0` nor `1`.
    BadOptional,
    /// A string field was not valid UTF-8.
    BadUtf8,
    /// A discriminator byte was unknown.
    UnknownTag(u8),
}

impl std::fmt::Display for ProtoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Truncated => write!(f, "truncated frame"),
            Self::BadVersion => write!(f, "missing protocol version"),
            Self::BadOptional => write!(f, "bad optional-field tag"),
            Self::BadUtf8 => write!(f, "invalid utf-8 in string field"),
            Self::UnknownTag(t) => write!(f, "unknown wire tag 0x{t:02x}"),
        }
    }
}

impl std::error::Error for ProtoError {}
