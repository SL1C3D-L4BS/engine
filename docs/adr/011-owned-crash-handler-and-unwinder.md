# ADR-011 — Owned crash handler and unwinder

- Status: Accepted
- Date: 2026-05-18 (expanded 2026-05-24 per audit §15 Phase-0 ADR sweep)
- Phase: 0 (pre-v1.0 — contract-exempt per risk R-03)
- Companion: ADR-010 (telemetry — postmortem replays the IPC
  buffer), ADR-030 (sampling profiler — same signal-handler
  discipline), Phase 10 (future realisation of `engine-postmortem`)

## Context

Game engines crash. The C++ legacy is grim: a SIGSEGV inside
the renderer typically calls `libunwind`, which heap-allocates
during stack walking, which fails because the heap may be
corrupt, which produces an unhelpful "Aborted (core dumped)"
and no actionable diagnostic.

Rust improves the situation (panics unwind cleanly, the standard
library's backtrace machinery is reliable for *Rust* panics), but
the engine has surface that bypasses the panic machinery: a
SIGSEGV from a `wgpu` driver crash, a SIGBUS from a misaligned
mmap, an out-of-memory abort from the system allocator. Rust's
panic infrastructure does not catch these.

The spec (§XIV) requires:

- A crash handler that does *not* allocate during signal handling.
- Backtraces from frame pointers — no libunwind in the signal
  path.
- A pre-allocated crash buffer (16 MiB) the handler writes to.
- A separate crash daemon that drains the buffer post-crash
  and assembles the postmortem; the daemon cannot crash
  because it does nothing while the engine is healthy.

## Decision

The engine ships an owned crash handler. The contract:

- **Signal handlers** (SIGSEGV, SIGBUS, SIGILL, SIGABRT, SIGFPE
  on Unix; the equivalents via `SetUnhandledExceptionFilter` on
  Windows). Installed at engine startup, before any allocation.
- **Frame-pointer unwinding.** The Rust build flag
  `-Cforce-frame-pointers=yes` is set workspace-wide; the
  handler walks `%rbp` (x86-64) / `x29` (aarch64) to produce a
  backtrace without calling libunwind or any allocator.
- **Pre-allocated crash buffer.** 16 MiB owned by the runtime,
  reserved at startup. The signal handler writes the backtrace
  + the last 1 s of telemetry events (ADR-010) + the
  scheduler's current frame state into this buffer. No
  allocation, no heap touch.
- **Crash daemon** — a small companion process (`engine-crashd`
  / `engine-postmortem` per spec Phase 10) connected to the
  engine via a Unix domain socket. The engine sends a
  "crashed, here is my buffer fd" message; the daemon reads
  the buffer and writes the postmortem report. The daemon
  does almost nothing while the engine is healthy, so it
  cannot crash.
- **No libunwind dependency.** The `backtrace` crate is not in
  the foundation layer. Stack walking is owned, ~200 lines, an
  exhaustively oracle-tested loop.

This ADR records the contract; Phase 10's `engine-postmortem`
realises the daemon side; the engine-side handler lands with
the editor or earlier (TBD; the spec's crash story is Phase 10
because the engine has nothing user-facing to crash before
then).

## Rationale

Three properties motivate owning the crash path:

1. **The crash path cannot allocate.** A crash inside a malloc
   that recursively calls SIGSEGV is the worst-case debugging
   scenario. The owned handler proves at compile time (no
   `Box::new`, no `Vec::push`, no `String::push_str` in the
   signal-safe path) that allocation does not happen.
2. **Frame-pointer unwinding is platform-agnostic.** libunwind
   is excellent on Linux but is itself C code with allocation
   risk in pathological cases. Frame-pointer walking, given
   `-Cforce-frame-pointers=yes`, is a tight loop the engine
   owns.
3. **Postmortem is a recording, not a reconstruction.** Because
   the engine writes the last 1 s of telemetry into the crash
   buffer before exit, the postmortem report can show *what
   the engine was doing* at the moment of crash, not just
   *where the crash occurred*. The signal context is the
   tip; the recording is the body.

The 16 MiB buffer is a generous size given typical crash
content; the buffer is pre-allocated and locked into RAM at
startup so even a page-fault under OOM cannot fail the handler.

## Consequences

- `-Cforce-frame-pointers=yes` is workspace-wide. Costs ~1
  register on aarch64 (negligible; x29 is callee-saved
  anyway), ~1 register on x86-64 (small; modern CPUs are not
  register-pressured outside vectorised hot loops).
- The crash handler depends on no dependencies outside the
  Rust core library — not even `std`-level allocation routines
  in the signal-safe scope.
- The 16 MiB buffer is part of the engine's startup memory
  footprint. Acceptable on every platform the engine targets.
- The daemon binary (`engine-postmortem`, Phase 10) is small;
  Phase 10 schedules its delivery alongside the editor.
- This ADR's contract is testable by a "deliberate crash"
  integration test that triggers SIGSEGV via a known route
  and verifies the daemon writes the expected report.

## Risks and tradeoffs

- **Frame-pointer reliability** in highly optimised LTO builds.
  Mitigation: the workspace's release profile sets
  `force-frame-pointers=yes` *and* the integration test runs
  in release mode; an LTO regression that breaks the
  backtrace fails CI.
- **Async-signal-safe constraint.** The signal handler can
  use only async-signal-safe functions; `write(2)`-equivalents
  for the buffer write, `kill(2)` for daemon notification.
  Mitigation: the signal-safe scope is a small `#[no_mangle]`
  function with documented allowed APIs.
- **Windows pattern differs.** Windows uses
  `SetUnhandledExceptionFilter` + minidump-style write; the
  same buffer-and-daemon pattern adapted to the platform.
  Tracked for Phase 10.
- **Telemetry consent.** Crash reports may be sent off-device
  for the developer's debugging; ADR-020 / GDPR / opt-in
  consent rules apply. Per-feature consent toggle:
  "send crash reports" is a separate consent from "send
  telemetry."

## Alternatives considered

- **Use `backtrace` crate / libunwind.** Allocation risk in
  the signal path; pulls a substantial dependency tree.
  Rejected.
- **Rust's `set_panic_hook` only** (no signal handler). Catches
  Rust panics; misses SIGSEGV/SIGBUS. Insufficient.
- **Use `breakpad` / `crashpad`.** Used by Chrome/Firefox;
  industry-grade. Doesn't fit the owned-foundation discipline;
  significant binary size; complexity exceeds the engine's
  needs. Rejected.
- **Re-exec via Watchdog.** A parent process re-launches the
  engine on crash. Useful pattern for live-ops resilience;
  orthogonal to the diagnostic capture problem this ADR
  solves; both can coexist in Phase 10.

## Verification

- The "deliberate crash" integration test (Phase 10):
  triggers SIGSEGV at a known site; verifies
  `engine-postmortem` produces a report containing the
  expected backtrace + the last second of telemetry.
- The signal-safe scope's discipline is enforced by code
  review (the audit's §11 will revisit Phase 10's
  realisation).
- Frame-pointer integrity is verified by a backtrace-of-known-
  function unit test that confirms a known call chain
  produces the expected symbolic frames.
- The 16 MiB buffer's pre-allocation is verified by a startup
  integration test (the engine's RSS includes the buffer
  before any work begins).
