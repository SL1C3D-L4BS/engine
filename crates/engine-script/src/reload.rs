//! Hot-reload (spec IV.7, ADR-036).
//!
//! A `Reloader` wraps a `FileWatcher` (the existing
//! `engine_platform::watch::InotifyWatcher` on Linux,
//! `PollingWatcher` elsewhere — Phase 4 PR 3 adds **no new file-watch
//! code**, it just consumes the trait) and a `Vm`. On every watched
//! `.sli`/`.bp` modification, the reloader re-runs the compiler and
//! atomically swaps the VM's module. Running fibers see the new
//! bytecode on the next `Call`; in-flight frames keep their function
//! id but execute the new code. `Event::ModuleReloaded` is emitted so
//! the debugger can re-arm line breakpoints.

use crate::ext::{SourceKind, classify};
use crate::{Compiler, Source, SourceMap, Vm};
use engine_platform::watch::{FileWatcher, WatchEvent, WatchKind};
use std::fs;
use std::path::{Path, PathBuf};

/// One reload-pipeline notification.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// `path` was modified and recompiled successfully. The VM now
    /// runs the new bytecode.
    ModuleReloaded {
        /// Source path.
        path: PathBuf,
    },
    /// `path` was modified but failed to recompile. The VM is
    /// unchanged.
    ModuleFailed {
        /// Source path.
        path: PathBuf,
        /// First compile-error message.
        message: String,
    },
}

/// Hot-reload pipeline: file watcher → compiler → module swap.
pub struct Reloader<W: FileWatcher> {
    watcher: W,
}

impl<W: FileWatcher> Reloader<W> {
    /// Constructs a reloader around an already-started watcher.
    pub fn new(watcher: W) -> Self {
        Self { watcher }
    }

    /// Drains pending file-system events and applies them to `vm`.
    /// Returns the list of generated reload events so the caller can
    /// fan them out to the debugger / log / editor.
    pub fn poll(&mut self, vm: &mut Vm) -> Vec<Event> {
        let mut events = Vec::new();
        for ev in self.watcher.poll() {
            if let Some(out) = self.handle(ev, vm) {
                events.push(out);
            }
        }
        events
    }

    fn handle(&self, ev: WatchEvent, vm: &mut Vm) -> Option<Event> {
        // Only `.sli` and `.bp` modifications trigger a reload.
        if !matches!(ev.kind, WatchKind::Modified | WatchKind::Created) {
            return None;
        }
        let kind = classify(ev.path.to_str().unwrap_or(""))?;
        match kind {
            SourceKind::SliCanonical | SourceKind::SliBpAlias => {}
        }
        let text = match fs::read_to_string(&ev.path) {
            Ok(t) => t,
            Err(e) => {
                return Some(Event::ModuleFailed {
                    path: ev.path.clone(),
                    message: format!("read failed: {e}"),
                });
            }
        };
        let mut sm = SourceMap::new();
        let id = sm.add(Source::new(ev.path.to_string_lossy().into_owned(), text));
        let compiled = match Compiler::new().compile(id, sm.get(id)) {
            Ok(c) => c,
            Err((_e, diags)) => {
                let message = diags
                    .all()
                    .iter()
                    .map(|d| d.message.clone())
                    .next()
                    .unwrap_or_else(|| "compile failed".to_string());
                return Some(Event::ModuleFailed {
                    path: ev.path.clone(),
                    message,
                });
            }
        };
        if compiled.diagnostics.has_errors() {
            let message = compiled
                .diagnostics
                .all()
                .iter()
                .find(|d| d.severity == crate::diag::Severity::Error)
                .map(|d| d.message.clone())
                .unwrap_or_else(|| "compile errors".to_string());
            return Some(Event::ModuleFailed {
                path: ev.path.clone(),
                message,
            });
        }
        // Atomic swap. Running fibers keep their function-id; the new
        // module's `functions` slot at that id holds the fresh code.
        vm.module = compiled.bytecode;
        Some(Event::ModuleReloaded { path: ev.path })
    }
}

/// Reads `path` and compiles it into a fresh `Vm`. Convenience wrapper
/// for tests and the REPL.
pub fn compile_to_vm(path: &Path) -> Result<Vm, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("read failed: {e}"))?;
    let mut sm = SourceMap::new();
    let id = sm.add(Source::new(path.to_string_lossy().into_owned(), text));
    let compiled = Compiler::new()
        .compile(id, sm.get(id))
        .map_err(|(e, _)| format!("{e}"))?;
    if compiled.diagnostics.has_errors() {
        return Err("compile errors".to_string());
    }
    Ok(Vm::new(compiled.bytecode))
}
