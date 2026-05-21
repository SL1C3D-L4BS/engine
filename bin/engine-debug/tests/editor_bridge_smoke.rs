//! Runs the `editor-bridge` example as a subprocess and asserts its
//! exit code. The example exercises every protocol request / response /
//! event variant end-to-end (ADR-036); a clean exit proves the
//! Phase-10 editor's wire surface is intact.

use std::process::Command;

#[test]
fn example_exits_cleanly() {
    let status = Command::new(env!("CARGO"))
        .args([
            "run",
            "--quiet",
            "-p",
            "engine-debug",
            "--example",
            "editor-bridge",
        ])
        .status()
        .expect("cargo run --example editor-bridge");
    assert!(status.success(), "editor-bridge exited with {status:?}");
}
