//! Debugger wire-protocol round-trip oracle (PR 3, ADR-036).
//!
//! Encodes every request, response, and event variant, decodes them
//! through `decode_frame`, and asserts equality. Catches version-byte
//! drift, tag-byte drift, and operand-encoding regressions.

use engine_script::BreakpointId;
use engine_script::debug_proto::{
    Event, FrameInfo, Local, PROTOCOL_VERSION, Request, Response, StopReason, WireValue,
    decode_event, decode_frame, decode_request, decode_response, encode_event, encode_request,
    encode_response,
};

fn roundtrip_req(req: Request) {
    let bytes = encode_request(&req);
    let (version, body, consumed) = decode_frame(&bytes).unwrap();
    assert_eq!(version, PROTOCOL_VERSION);
    assert_eq!(consumed, bytes.len());
    assert_eq!(decode_request(&body).unwrap(), req);
}

fn roundtrip_resp(resp: Response) {
    let bytes = encode_response(&resp);
    let (version, body, consumed) = decode_frame(&bytes).unwrap();
    assert_eq!(version, PROTOCOL_VERSION);
    assert_eq!(consumed, bytes.len());
    assert_eq!(decode_response(&body).unwrap(), resp);
}

fn roundtrip_event(ev: Event) {
    let bytes = encode_event(&ev);
    let (version, body, consumed) = decode_frame(&bytes).unwrap();
    assert_eq!(version, PROTOCOL_VERSION);
    assert_eq!(consumed, bytes.len());
    assert_eq!(decode_event(&body).unwrap(), ev);
}

#[test]
fn every_request_roundtrips() {
    roundtrip_req(Request::SetBreakpoint {
        file_id: 1,
        line: 42,
        condition: Some("x > 0".into()),
        hit_count: Some(3),
    });
    roundtrip_req(Request::SetBreakpoint {
        file_id: 1,
        line: 42,
        condition: None,
        hit_count: None,
    });
    roundtrip_req(Request::SetFunctionBreakpoint {
        fn_name: "main".into(),
    });
    roundtrip_req(Request::SetExceptionBreakpoint { mask: 0xFFFF });
    roundtrip_req(Request::ClearBreakpoint {
        id: BreakpointId(7),
    });
    roundtrip_req(Request::Continue);
    roundtrip_req(Request::Pause);
    roundtrip_req(Request::StepOver);
    roundtrip_req(Request::StepInto);
    roundtrip_req(Request::StepOut);
    roundtrip_req(Request::RunToCursor {
        file_id: 1,
        line: 99,
    });
    roundtrip_req(Request::ListFrames);
    roundtrip_req(Request::ListLocals { frame_id: 0 });
    roundtrip_req(Request::ExpandValue {
        frame_id: 0,
        path: "r0.x".into(),
    });
    roundtrip_req(Request::SetLocal {
        frame_id: 0,
        name: "r0".into(),
        value: WireValue::Int(42),
    });
    roundtrip_req(Request::Watch {
        expr: "x + 1".into(),
    });
    roundtrip_req(Request::Unwatch { id: 3 });
    roundtrip_req(Request::EvalConst {
        expr: "2 * 3".into(),
    });
    roundtrip_req(Request::Detach);
}

#[test]
fn every_response_roundtrips() {
    roundtrip_resp(Response::Ack);
    roundtrip_resp(Response::BreakpointId(BreakpointId(7)));
    roundtrip_resp(Response::WatchId(3));
    roundtrip_resp(Response::ConstValue(WireValue::Int(5)));
    roundtrip_resp(Response::Frames(vec![FrameInfo {
        index: 0,
        function: "main".into(),
        line: 5,
    }]));
    roundtrip_resp(Response::Locals(vec![Local {
        name: "r0".into(),
        reg: 0,
        value: WireValue::Int(42),
        dirty: true,
    }]));
    roundtrip_resp(Response::Value(WireValue::Str("hi".into())));
    roundtrip_resp(Response::Error("boom".into()));
}

#[test]
fn every_event_roundtrips() {
    roundtrip_event(Event::Stopped {
        reason: StopReason::Breakpoint,
        fiber_id: 0,
        frame_id: 0,
    });
    roundtrip_event(Event::BreakpointHit {
        id: BreakpointId(7),
        fiber_id: 0,
    });
    roundtrip_event(Event::Exception {
        fiber_id: 0,
        message_id: 12,
    });
    roundtrip_event(Event::OutputLine {
        stream: 0,
        text: "hello\n".into(),
    });
    roundtrip_event(Event::ModuleReloaded { file_id: 1 });
    roundtrip_event(Event::WatchUpdate {
        id: 0,
        value: WireValue::Int(42),
    });
}

#[test]
fn version_byte_is_protocol_version() {
    let bytes = encode_request(&Request::Continue);
    // The 4-byte length prefix is followed by a 2-byte LE proto version.
    let v = u16::from_le_bytes([bytes[4], bytes[5]]);
    assert_eq!(v, PROTOCOL_VERSION);
}
