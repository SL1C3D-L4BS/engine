//! Hot-reload oracle (PR 3, ADR-036).
//!
//! Writes a `.sli` module to a tempdir, runs it under the polling
//! file watcher, rewrites the file, and asserts the VM sees the new
//! bytecode on the next call. `Event::ModuleReloaded` is observed.

use engine_platform::watch::PollingWatcher;
use engine_script::reload::{Event, Reloader};
use engine_script::{StopReason, Value, reload};
use std::fs;
use std::thread::sleep;
use std::time::Duration;

#[test]
fn modified_module_swaps_into_vm() {
    let dir = tempdir();
    let path = dir.path().join("main.sli");
    fs::write(&path, "fn pick() -> i64 { return 1; }").unwrap();

    let mut vm = reload::compile_to_vm(&path).unwrap();
    assert_eq!(vm.call("pick", vec![]), StopReason::Returned(Value::Int(1)));

    // Sleep so the polling watcher snapshot differs after rewrite.
    sleep(Duration::from_millis(20));
    let mut reloader = Reloader::new(PollingWatcher::new(dir.path()));
    // Prime the watcher's snapshot so subsequent events are recorded.
    let _ = reloader.poll(&mut vm);

    sleep(Duration::from_millis(20));
    fs::write(&path, "fn pick() -> i64 { return 99; }").unwrap();
    sleep(Duration::from_millis(20));

    let events = reloader.poll(&mut vm);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::ModuleReloaded { .. })),
        "expected ModuleReloaded, got {events:?}",
    );
    assert_eq!(
        vm.call("pick", vec![]),
        StopReason::Returned(Value::Int(99))
    );
}

struct TempDir(std::path::PathBuf);

impl TempDir {
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn tempdir() -> TempDir {
    let mut p = std::env::temp_dir();
    let suffix = format!(
        "engine-script-reload-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    p.push(suffix);
    std::fs::create_dir_all(&p).unwrap();
    TempDir(p)
}
