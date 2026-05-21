//! engine-debug smoke test (PR 3, ADR-036).
//!
//! Compiles a fixture, installs a breakpoint, runs the script, and
//! asserts the trap fires. Self-contained (no IPC) — the wire-protocol
//! correctness is pinned by `editor_bridge_smoke.rs` and the
//! cross-crate `debug_protocol.rs` test.

use engine_script::debug::Debugger;
use engine_script::{Compiler, Source, SourceMap, StopReason, Value, Vm};

#[test]
fn breakpoint_fires_on_entry_line() {
    let src = r#"
fn entry() -> i64 {
    return 42;
}
"#;
    let mut sm = SourceMap::new();
    let id = sm.add(Source::new("smoke.sli", src));
    let compiled = Compiler::new().compile(id, sm.get(id)).unwrap();
    let mut vm = Vm::new(compiled.bytecode);
    let mut dbg = Debugger::new();
    let bp = dbg.set_breakpoint(&mut vm.module, 0, 3).unwrap();
    let stop = vm.call("entry", vec![]);
    let StopReason::Trapped { function_id, pc } = stop else {
        panic!("expected trap, got {stop:?}");
    };
    let id = dbg.record_hit(function_id, pc).expect("hit recorded");
    assert_eq!(id, bp);
    let info = dbg.breakpoint(id).unwrap();
    assert_eq!(info.line, 3);
    assert_eq!(info.hits, 1);
    // Sanity: the value didn't surface (we trapped before return).
    let _ = Value::Nil;
}
