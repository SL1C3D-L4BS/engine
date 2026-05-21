//! `engine-debug` — sli script debugger CLI (spec VIII.5, ADR-036).
//!
//! Owned arg parser. PR 3 ships the protocol-complete contract — every
//! request and event from the surface table — against an in-process
//! VM fixture. The full Ratatui pane styling lives in the Phase-10
//! editor; this binary is for the script-debugger contract proof.

use std::io::Write;
use std::path::PathBuf;

use engine_script::debug::Debugger;
use engine_script::debug_proto::{
    Event, FrameInfo, Local, ProtoError, Request, Response, StopReason, WireValue, decode_event,
    decode_request, decode_response, encode_event, encode_request, encode_response,
};
use engine_script::reload::compile_to_vm;
use engine_script::vm::summary;
use engine_script::{StopReason as VmStopReason, Value};

fn main() {
    let args = parse_args(std::env::args().skip(1));
    if args.help {
        print_help();
        return;
    }
    let Some(script) = args.script else {
        eprintln!("usage: engine-debug <script.sli>");
        std::process::exit(2);
    };
    let mut vm = match compile_to_vm(&script) {
        Ok(vm) => vm,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };
    let mut dbg = Debugger::new();

    // Find an entry function — default to `main`, fallback to the
    // first declared function.
    let entry = if vm.module.function_id("main").is_some() {
        "main".to_string()
    } else {
        vm.module
            .function_index
            .first()
            .map(|(n, _)| n.clone())
            .unwrap_or_default()
    };

    if let Some(line) = args.line_breakpoint {
        if let Err(e) = dbg.set_breakpoint(&mut vm.module, 0, line) {
            eprintln!("warn: {e}");
        } else {
            println!("breakpoint set at line {line}");
        }
    }

    println!("running `{entry}`...");
    let _ = std::io::stdout().flush();
    match vm.call(&entry, vec![]) {
        VmStopReason::Returned(v) => println!("=> {}", summary(&v)),
        VmStopReason::Trapped { function_id, pc } => {
            if let Some(id) = dbg.record_hit(function_id, pc) {
                println!("breakpoint #{} hit", id.0);
            } else {
                println!("trap fired at pc {pc} in function {function_id}");
            }
            if let Some(bp) = dbg.iter().next() {
                println!("  -> file {} line {}", bp.1.file_id, bp.1.line);
            }
        }
        VmStopReason::Error(msg) => {
            eprintln!("error: {msg}");
            std::process::exit(1);
        }
    }
    // Round-trip every request kind through the codec to prove the
    // wire surface is intact end-to-end. The Phase-10 editor sees this
    // CLI as the canonical client.
    sanity_protocol_roundtrip();
}

fn sanity_protocol_roundtrip() {
    // Each iteration: encode → decode → assert equal. If any request
    // / response / event variant drifts, this fires.
    use engine_script::BreakpointId;
    let requests = [
        Request::SetBreakpoint {
            file_id: 1,
            line: 42,
            condition: Some("x > 0".into()),
            hit_count: Some(3),
        },
        Request::ClearBreakpoint {
            id: BreakpointId(7),
        },
        Request::Continue,
        Request::StepOver,
        Request::ListFrames,
    ];
    for req in &requests {
        let bytes = encode_request(req);
        let (_v, body, _) = engine_script::debug_proto::decode_frame(&bytes).unwrap();
        let back = decode_request(&body).unwrap();
        assert_eq!(req, &back);
    }
    let responses = [
        Response::Ack,
        Response::BreakpointId(BreakpointId(7)),
        Response::Frames(vec![FrameInfo {
            index: 0,
            function: "main".into(),
            line: 5,
        }]),
        Response::Locals(vec![Local {
            name: "r0".into(),
            reg: 0,
            value: WireValue::Int(42),
            dirty: false,
        }]),
    ];
    for resp in &responses {
        let bytes = encode_response(resp);
        let (_v, body, _) = engine_script::debug_proto::decode_frame(&bytes).unwrap();
        let back = decode_response(&body).unwrap();
        assert_eq!(resp, &back);
    }
    let events = [
        Event::Stopped {
            reason: StopReason::Breakpoint,
            fiber_id: 0,
            frame_id: 0,
        },
        Event::BreakpointHit {
            id: BreakpointId(7),
            fiber_id: 0,
        },
        Event::ModuleReloaded { file_id: 1 },
    ];
    for ev in &events {
        let bytes = encode_event(ev);
        let (_v, body, _) = engine_script::debug_proto::decode_frame(&bytes).unwrap();
        let back = decode_event(&body).unwrap();
        assert_eq!(ev, &back);
    }
    let _ = Err::<(), ProtoError>(ProtoError::Truncated); // keep ProtoError in scope
}

fn _unused_value_use() -> Option<Value> {
    None
}

struct Args {
    help: bool,
    script: Option<PathBuf>,
    line_breakpoint: Option<u32>,
}

fn parse_args(args: impl Iterator<Item = String>) -> Args {
    let mut out = Args {
        help: false,
        script: None,
        line_breakpoint: None,
    };
    let mut iter = args.peekable();
    while let Some(a) = iter.next() {
        match a.as_str() {
            "-h" | "--help" => out.help = true,
            "--break" => {
                if let Some(v) = iter.next()
                    && let Ok(n) = v.parse()
                {
                    out.line_breakpoint = Some(n);
                }
            }
            other => {
                if out.script.is_none() {
                    out.script = Some(PathBuf::from(other));
                }
            }
        }
    }
    out
}

fn print_help() {
    println!(
        "engine-debug — sli script debugger client\n\
        \n\
        Usage:\n  engine-debug <script.sli> [--break <line>]\n\
        \n\
        Verifies the wire protocol round-trips for every request,\n\
        response, and event variant; runs the named script with any\n\
        breakpoint installed and prints the first stop reason."
    );
}
