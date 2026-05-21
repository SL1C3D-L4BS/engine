# ADR-036 · sli hot-reload, debugger protocol, and REPL

- Status: Accepted (PR 3 of Phase 4)
- Date: 2026-05-20
- Phase: 4 (SCRIPTING — spec Part XXI)

## Context

ADR-034 shipped the sli compiler front-end. ADR-035 shipped the
register VM, GC, FFI, and verifier. PR 3 closes the spec IV.7 / VIII.4 /
VIII.5 / XII surface:

- **Hot-reload** of `.sli` (and `.bp` legacy alias) modules through the
  existing `engine_platform::watch` `FileWatcher` trait (Phase 3
  inotify + polling backends — PR 3 introduces *no new file-watch
  code*).
- A **debugger** with a TRAP-opcode-based breakpoint mechanism, an
  owned binary wire protocol that ships every request and event from
  spec XII, watch-expression purity verification, and `(file, line)`
  breakpoint persistence to `.engine/debug/breakpoints.toml`.
- A **REPL** (line-editor front-end + program-per-line evaluator) with
  the spec VIII.4 dot-command set.
- Two new CLIs — `bin/engine-repl` and `bin/engine-debug` — that wire
  the above to stdin/stdout so the Phase-10 editor inherits a
  protocol-complete debugger contract.

The umbrella editor UI, raw-mode termios layer, and full TUI keybind
set remain Phase-10 work; PR 3 ships the protocol and the data plane,
not the surface chrome.

## Decision

### Hot-reload (`crates/engine-script/src/reload.rs`)

`Reloader<W: FileWatcher>` wraps the existing watcher trait and a
`Vm`. On every watched modification, the reloader re-runs the
compiler, *verifies* the new bytecode, and **atomically swaps** the
VM's module on success. In-flight frames keep their `function_id`
but the next `Call` sees the swapped code; running fibers do not see
torn bytecode because the swap is a single move of `Vm::module`. On
a compile failure, the old module stays installed and the reloader
emits `Event::ReloadFailed { diagnostics }`. The debugger's `rearm()`
hook re-walks the new module's `line_for_pc` table and re-installs
every `(file, line)` breakpoint, dropping any whose lines no longer
map to opcode boundaries.

`Event::ModuleReloaded { file_id }` and `Event::ReloadFailed
{ diagnostics }` are the two new payloads. They feed both the
debugger event stream (for the editor's "[RELOADED]" badge in the
Phase-10 BLUEPRINTS pane) and `engine_core::telemetry::Signal`
(`ScriptBreakpointHit` + `ScriptException` shipped here so the wider
telemetry stack already counts script-side incidents — the
metric names `sli_script_breakpoint_hits_total` and
`sli_script_exceptions_total` join the engine-telemetry counter set).

### Debugger (`debug.rs` + `debug_proto.rs` + `watch_expr.rs`)

**Breakpoint mechanism.** The debugger patches the target opcode byte
with `Opcode::Trap` (`0xFF`). On hit, dispatch loops out of the inner
loop with `StopReason::Trapped { function_id, pc }`; the controlling
client looks up the breakpoint by `(function_id, pc)`, restores the
original byte for stepping, and re-patches if the breakpoint should
persist. The TRAP byte is reserved by ADR-035's four-layer
impossibility argument — user code cannot collide with it.

**Wire protocol.** Owned binary, **not** Microsoft DAP JSON. Each
frame is `[u32 LE body length][u16 LE proto_version][u8 kind][body]`,
where `proto_version = 0x0001` ships in v0.4. The envelope reuses
`engine_telemetry::ipc::write_frame` / `read_frame` so the wire layer
is identical to the rest of the engine's IPC. Tag bytes:

| range | family |
| ----- | ------ |
| `0x10..` | requests (`Continue`, `StepOver`, `StepInto`, `StepOut`, `Pause`, `EvaluateWatch`, `ListFrames`, `ListLocals`, `ExpandValue`, `SetBreakpoint`, `ClearBreakpoint`, `Attach`, `Detach`) |
| `0x40..` | responses |
| `0x80..` | events (`Stopped`, `Continued`, `OutputLine`, `ModuleReloaded`, `ExceptionThrown`) |

**No serde, no JSON.** The CI guard in this PR rejects `serde`,
`serde_json`, `serde_derive`, `bincode`, `rmp`, `prost`, and
`protobuf` anywhere under `crates/engine-script/`. The protocol
encoder lives in `debug_proto.rs`; it owns its own little-endian
operand encoding and a round-trip oracle pins every variant.

**Watch expressions.** `watch_expr::validate(expr)` walks the parsed
AST and rejects every form that could mutate state or call non-pure
code (`Assign`, `Let`, `While`, function calls outside an
allowlist of pure stdlib builtins). The allowlist ships *empty* in
PR 3 — stdlib `Math.*` / `Str.*` builtins enter it as they land —
which is the strict-deny default the spec calls for. Evaluation
piggy-backs on `crate::consteval` so the in-paused-frame execution
path is exactly the const-expression interpreter.

**Persistence.** Spec XII calls for RON. The repo has no RON
parser, the manifest layer (`engine.toml`) is already TOML, and
adding a RON dependency for a six-field table is a side-quest.
PR 3 ships `.engine/debug/breakpoints.toml` with an owned ~80 LoC
write-only TOML keyer that round-trips through a flat key=value
reader. This deviation joins the `[[foundation-layer-deviations]]`
tradition and is recorded in this ADR.

### REPL (`repl.rs` + `bin/engine-repl`)

`Repl` owns a single persistent `Module` across inputs. Each input
line (or multi-line balanced bracket group — `unmatched_brackets`
counts braces/brackets/parens to detect continuation) is wrapped in
a synthesised `fn _repl() -> ...` and pumped through the same
front-end as a regular module. Dot-commands match spec VIII.4:

- `.help` `.exit` `.clear`
- `.history` (in-process; the CLI binary persists it to a session
  file)
- `.type <expr>` (runs typeck and prints the inferred type)
- `.bytecode <expr>` (lowers and disassembles via the existing
  `Opcode::Display` impl)
- `--attach` mode wires `.ecs`, `.profile`, `.asset` to the running
  engine; PR 3 ships the dispatcher, the attach session is Phase 10.

The line editor in `bin/engine-repl` is cooked-mode stdin. A
raw-mode termios layer is the Phase-10 editor's job; PR 3 stays
portable and works under non-tty stdin (test harnesses, CI).

### Determinism

- The hot-reload oracle (`tests/hot_reload.rs`) drives the polling
  watcher backend (deterministic across platforms — the inotify
  backend is event-driven and not suited to unit testing).
- The debugger wire-protocol round-trip oracle
  (`tests/debug_protocol.rs`) encodes every request, response, and
  event variant, decodes, and asserts equality — cross-arch parity
  rides on every operand being little-endian by construction.
- The breakpoint-persistence oracle
  (`tests/breakpoint_persistence.rs`) round-trips through the owned
  TOML writer + reader.
- The watch-expression safety oracle
  (`tests/watch_expr_safety.rs`) pins the strict-deny default.

### CI guards added in this PR

- **Owned debugger wire protocol** — `serde`, `serde_json`,
  `serde_derive`, `bincode`, `rmp`, `prost`, `protobuf` are
  rejected under `crates/engine-script/` (extends the ADR-034
  grep guard).
- **Owned line editor** — `rustyline`, `reedline`, `linefeed`
  rejected under `bin/engine-repl/`. The Phase-10 editor will
  wire its own termios layer; until then PR 3 stays line-mode.

## Consequences

### Positive

- Hot-reload, debugger, and REPL ship without any new third-party
  runtime dependency. The grep guard is auditable.
- The debugger wire protocol is binary, versioned, and reuses the
  telemetry envelope — the editor speaks one transport for both
  signal streams.
- The protocol round-trip oracle is the proof Phase-10 editor work
  is UI-only — every affordance is settled here.
- TRAP-based breakpoints inherit the ADR-035 four-layer
  impossibility argument: user code cannot accidentally trigger a
  breakpoint trap.

### Negative / accepted tradeoffs

- **TOML, not RON.** Recorded above; matches platform manifests.
- **Line-mode REPL.** Raw-mode editor is Phase-10 work.
- The PR-3 single-generation GC inherits ADR-035's pause-time
  caveat: hot-reload of a 100k-object program runs with the
  major-GC budget until the generational variant lands.

## Owner

Sliced Engine team. PR 3 in the Phase-4 sequence; ADR-037
(toolchain shape) and ADR-038 (Slang reproducibility) close the
phase.
