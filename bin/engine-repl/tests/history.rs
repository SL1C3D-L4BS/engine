//! REPL history oracle (PR 3, ADR-036).
//!
//! `Repl::history()` returns the accumulated input list; the REPL
//! binary persists it through `save_history`. The oracle here pins
//! the Repl-side accumulation (the on-disk persistence in
//! `bin/engine-repl/src/main.rs` is straightforward `fs::write`).

use engine_script::Repl;
use engine_script::repl::Reply;

#[test]
fn history_accumulates_evaluated_inputs() {
    let mut r = Repl::new();
    let _ = r.eval("1 + 2");
    let _ = r.eval("3 * 4");
    assert_eq!(r.history(), &["1 + 2".to_string(), "3 * 4".to_string()]);
}

#[test]
fn dot_commands_do_not_pollute_history() {
    let mut r = Repl::new();
    let _ = r.eval(".help");
    let _ = r.eval("1 + 1");
    let _ = r.eval(".clear");
    let _ = r.eval("2 + 2");
    // `.clear` empties the history, then `2 + 2` is the only entry.
    assert_eq!(r.history(), &["2 + 2".to_string()]);
}

#[test]
fn exit_command_returns_exit_reply() {
    let mut r = Repl::new();
    assert!(matches!(r.eval(".exit"), Reply::Exit));
}
