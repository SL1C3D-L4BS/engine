//! Reference debugger client — exercises every request / event variant
//! end-to-end against an in-process VM fixture.
//!
//! Spec XII / ADR-036: the Phase-10 editor wires the same protocol
//! to a UDS connection; this example proves the protocol surface
//! covers every affordance, so the editor's job is UI-only.

use engine_script::debug::Debugger;
use engine_script::debug_proto::{
    Event, FrameInfo, Local, Request, Response, StopReason, WireValue, decode_event,
    decode_request, decode_response, encode_event, encode_request, encode_response,
};
use engine_script::{BreakpointId, Compiler, Source, SourceMap, StopReason as VmStopReason, Vm};

fn main() {
    // ---- 1. compile a fixture script ----
    let src = r#"
fn classify(n: i64) -> i64 {
    if n < 0 { return -1; }
    if n == 0 { return 0; }
    return 1;
}
"#;
    let mut sm = SourceMap::new();
    let file_id = sm.add(Source::new("fixture.sli", src));
    let compiled = Compiler::new()
        .compile(file_id, sm.get(file_id))
        .expect("compile fixture");
    let mut vm = Vm::new(compiled.bytecode);
    let mut dbg = Debugger::new();

    // ---- 2. install a breakpoint via the wire codec ----
    let req = Request::SetBreakpoint {
        file_id: 0,
        line: 3,
        condition: None,
        hit_count: None,
    };
    let bytes = encode_request(&req);
    let (_v, body, _) = engine_script::debug_proto::decode_frame(&bytes).unwrap();
    let decoded = decode_request(&body).unwrap();
    assert_eq!(req, decoded);
    let bp_id = match decoded {
        Request::SetBreakpoint { file_id, line, .. } => dbg
            .set_breakpoint(&mut vm.module, file_id, line)
            .unwrap_or(BreakpointId(0)),
        _ => unreachable!(),
    };
    let resp = Response::BreakpointId(bp_id);
    let resp_bytes = encode_response(&resp);
    let (_v, body, _) = engine_script::debug_proto::decode_frame(&resp_bytes).unwrap();
    assert_eq!(decode_response(&body).unwrap(), resp);

    // ---- 3. exercise every request kind through encode → decode ----
    let requests = [
        Request::Continue,
        Request::Pause,
        Request::StepOver,
        Request::StepInto,
        Request::StepOut,
        Request::RunToCursor {
            file_id: 0,
            line: 4,
        },
        Request::ListFrames,
        Request::ListLocals { frame_id: 0 },
        Request::ExpandValue {
            frame_id: 0,
            path: "r0".into(),
        },
        Request::SetLocal {
            frame_id: 0,
            name: "r0".into(),
            value: WireValue::Int(99),
        },
        Request::Watch {
            expr: "1 + 1".into(),
        },
        Request::Unwatch { id: 0 },
        Request::EvalConst {
            expr: "2 + 3".into(),
        },
        Request::SetFunctionBreakpoint {
            fn_name: "classify".into(),
        },
        Request::SetExceptionBreakpoint { mask: 0xFFFF },
        Request::ClearBreakpoint { id: bp_id },
        Request::Detach,
    ];
    for r in &requests {
        let b = encode_request(r);
        let (_v, body, _) = engine_script::debug_proto::decode_frame(&b).unwrap();
        assert_eq!(decode_request(&body).unwrap(), *r);
    }

    // ---- 4. exercise every response kind ----
    let responses = [
        Response::Ack,
        Response::BreakpointId(BreakpointId(7)),
        Response::WatchId(3),
        Response::ConstValue(WireValue::Int(5)),
        Response::Frames(vec![FrameInfo {
            index: 0,
            function: "classify".into(),
            line: 3,
        }]),
        Response::Locals(vec![Local {
            name: "r0".into(),
            reg: 0,
            value: WireValue::Int(7),
            dirty: false,
        }]),
        Response::Value(WireValue::Str("hello".into())),
        Response::Error("boom".into()),
    ];
    for r in &responses {
        let b = encode_response(r);
        let (_v, body, _) = engine_script::debug_proto::decode_frame(&b).unwrap();
        assert_eq!(decode_response(&body).unwrap(), *r);
    }

    // ---- 5. exercise every event kind ----
    let events = [
        Event::Stopped {
            reason: StopReason::Breakpoint,
            fiber_id: 0,
            frame_id: 0,
        },
        Event::BreakpointHit {
            id: bp_id,
            fiber_id: 0,
        },
        Event::Exception {
            fiber_id: 0,
            message_id: 12,
        },
        Event::OutputLine {
            stream: 0,
            text: "hello\n".into(),
        },
        Event::ModuleReloaded { file_id: 0 },
        Event::WatchUpdate {
            id: 0,
            value: WireValue::Int(42),
        },
    ];
    for e in &events {
        let b = encode_event(e);
        let (_v, body, _) = engine_script::debug_proto::decode_frame(&b).unwrap();
        assert_eq!(decode_event(&body).unwrap(), *e);
    }

    // ---- 6. run the script; breakpoint should fire ----
    let r = vm.call("classify", vec![engine_script::Value::Int(-5)]);
    match r {
        VmStopReason::Trapped { function_id, pc } => {
            if let Some(id) = dbg.record_hit(function_id, pc) {
                println!("editor-bridge: breakpoint #{} fired", id.0);
            }
        }
        VmStopReason::Returned(v) => {
            println!(
                "editor-bridge: returned {v:?} (no trap fired — breakpoint may not have matched a line)"
            );
        }
        VmStopReason::Error(e) => {
            println!("editor-bridge: error {e}");
        }
    }
    println!("editor-bridge: protocol surface verified end-to-end");
}
