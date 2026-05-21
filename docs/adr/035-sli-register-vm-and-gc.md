# ADR-035 · sli register VM, tri-color GC, and bytecode verifier

- Status: Accepted (PR 2 of Phase 4)
- Date: 2026-05-20
- Phase: 4 (SCRIPTING — spec Part XXI)

## Context

ADR-034 shipped the sli compiler front-end up to optimised SSA IR. PR 2
ships the executable substrate:

- a register-based bytecode the lowering pass emits,
- a verifier that gates execution,
- a register VM with threaded dispatch,
- a mark-and-sweep GC heap for aggregates (strings, structs, arrays,
  maps, closures),
- an FFI call table for Rust↔sli bridging,
- and `impl Asset for ScriptModule` so compiled modules ride the
  content-addressed pak pipeline (ADR-008).

The spec's IV.7 sub-millisecond GC pause budget and the Crafting
Interpreters generational design (nursery + old gen + remembered
set + parallel marker) remain the target. PR 2 lands the
*architecture* — module layout, public API, write-barrier hook call
site — but the implementation runs **single-generation** so the
oracle, the VM, and the rest of Phase 4 (hot-reload, debugger, REPL,
Slang) can land while the generational machinery is built out.

## Decision

### Bytecode

`crates/engine-script/src/bytecode.rs` defines `enum Opcode` as
`#[repr(u8)]`. **`TRAP = 0xFF` is the only variant whose discriminant
is `0xFF`.** A `const _: () = assert!(Opcode::Trap as u8 == 0xFF);`
pins that invariant at compile time; the verifier rejects any code
byte equal to `0xFF` (Layer 3 of the four-layer TRAP-collision
defence ADR-034 established as a target). Layer 4 is the runtime
fuzz oracle in `tests/codegen_no_trap.rs`, which compiles 500
deterministic synthetic programs alongside the PR-1 fixture corpus
and asserts every opcode byte is well-known.

The on-disk format is little-endian throughout: an 8-byte magic, a
4-byte version, the const pool, and the function table. PR 2 does not
yet need ADR-013 cross-arch byte-equality for the encoded module
itself — the VM-oracle golden (`tests/vm_oracle.rs`, BLAKE3 over
`(name, computed result)` pairs) pins cross-arch parity on what the
VM produces, which is the property the user-facing contract names.

### Register VM

`vm/mod.rs` exposes `Vm::call(name, args)`, which builds the
outermost `CallFrame` and runs `dispatch::run`. The dispatch loop
matches on the opcode byte with `#[inline(always)]` arms so a
release build folds it into a tightly-packed jump table — the
portable Rust equivalent of the GCC computed-goto trick the spec
calls out.

Calls push a new frame; `ReturnNil` / `ReturnVal` pop one. FFI
invocations route through `CallTable::call` and never touch the VM
stack.

### Garbage collector

`gc/mod.rs` is a tri-color mark-and-sweep heap. Allocations bump
into `Heap::objects` (slot table) or recycle from `free_list`. The
mark phase walks `roots` and follows handles in arrays / maps /
struct slots / closure upvalues; the sweep phase frees white slots
and adds them to the free list.

`GcConfig::tick_budget_us` defaults to **250 µs**, matching the spec
hint for the generational variant. The submodules `nursery.rs`,
`old_gen.rs`, `remembered.rs`, `barrier.rs` are *roadmap stubs* — the
public types exist so the eventual generational implementation can
drop in without touching call sites. `write_barrier_hook` is a no-op
today (single-gen heap has no cross-gen pointers) but is called from
the dispatch loop on handle stores so PR-3+ can light it up.

### Verifier

`verify.rs::verify(&Module)` walks every function and rejects:

- unknown opcode bytes,
- **any `0xFF` byte** (the architectural TRAP backstop),
- register operands `>= max_register`,
- const-pool / function-id operands out of table bounds,
- truncated instructions (operand bytes past EOF),
- jump targets out of range or off an opcode boundary,
- functions that fall off the end without a return.

Stack-balance and type-tag consistency the design also names ride on
the type checker (PR 1) — the verifier is the *execution* gate; the
type checker is the *meaning* gate. Both must pass.

### FFI

`ffi.rs::CallTable` registers Rust callbacks with a name, an optional
arity, and a `fn(&[Value], &mut Heap) -> Result<Value, String>`
signature. The PR-2 marshal layer is intentionally raw — `Value` in,
`Value` out — so the dispatch loop's `FfiCall` opcode can be a
single function-call indirection.

PR 3 will widen this with reflection-driven `ScriptArg` / `ScriptRet`
traits in `engine-reflect` so `Query<T>` / `Res<T>` / `ResMut<T>`
auto-register from the type registry. PR 2 ships the raw layer so the
FFI round-trip oracle (`tests/ffi.rs`) can prove every primitive
variant marshals correctly.

### Asset integration

`asset.rs::ScriptModule` implements `engine_asset::Asset`. Decoding
runs `bytecode::decode` over the bytes the asset server hands us; the
zero-copy `BlobSource::Mapped` borrow stays in scope through the
`Asset::decode` call, but PR 2 unconditionally copies string constants
into owned `String`s. The string-borrowing optimisation lands in PR 3
together with the hot-reload swap path.

## Cross-architecture determinism

The new `tests/vm_oracle.rs` BLAKE3 digest over execution results
joins the Phase-3 determinism matrix on x86-64 and aarch64 (CI patch
in this PR). The corpus is hand-picked from the PR-1 fixtures: pure
arithmetic, control-flow, recursion, logic, and clamp programs whose
expected values are byte-stable. Struct / closure / string-builtin
cases that need heap allocation are deferred to PR 3 when struct
construction lands as a first-class opcode.

## What PR 2 deliberately defers

- **Generational separation** — `Nursery`, `OldGen`, `RememberedSet`
  exist as roadmap types; the single-gen `Heap` is the live
  implementation. The pause-oracle (`tests/gc_pause_oracle.rs`) runs
  but is **informational** in PR 2; it becomes a hard CI gate when the
  generational variant lands.
- **Parallel marking** — single-threaded mark; ADR-032's owned thread
  pool stays unused by the GC for now.
- **Struct / array / map / closure opcodes** — the type checker
  validates these shapes (PR 1) and the heap can hold them (PR 2),
  but the codegen lowers them to `ConstNil` placeholders for PR 2.
  The literal-value oracles in `vm_oracle.rs` exercise the arithmetic
  / control-flow paths that *do* land.
- **engine-reflect `ScriptArg` / `ScriptRet` traits** — the raw
  `(args: &[Value], heap)` FFI signature is sufficient for PR 2;
  reflection-driven auto-registration is PR 3.

These deferrals are each tracked in the file that should grow them
(comments at the call site). The bones — `GcConfig::tick_budget_us`,
`write_barrier_hook` call site, module layout `gc/{nursery, old_gen,
remembered, barrier}.rs` — are in place so the follow-up work is
additive, not a rewrite.

## Test surface

- `tests/vm_oracle.rs` — committed BLAKE3 golden over the eleven
  execution cases; cross-arch via the determinism matrix.
- `tests/verifier.rs` — table-driven negative cases (TRAP, unknown
  op, OOB register / const / function id, truncated, bad jump,
  missing return) + a positive sanity case.
- `tests/gc_oracle.rs` — reachability correctness across keep / drop
  / transitive-keep / 100k-churn / slot-recycle scenarios.
- `tests/codegen_no_trap.rs` — corpus + 500-program fuzz oracle that
  no compiled function emits a `0xFF` opcode byte; each program is
  also re-verified end-to-end.
- `tests/ffi.rs` — register, dispatch, arity mismatch, primitive
  round-trip.
- `tests/gc_pause_oracle.rs` — p99 / max pause histogram over
  STEADY_STATE=100k, CHURN=10k/tick, TICKS=1k. Informational in PR 2;
  becomes a hard CI gate with the generational follow-up.

## Consequences

- PR 3 plugs into a working VM: hot-reload swaps a `Bytecode` on the
  `Vm`; the debugger patches `Opcode::Trap` opcodes at line addresses
  the `DebugInfo::functions` / `FunctionBytecode::line_for_pc`
  side-tables expose.
- The Slang toolchain in PR 4 is unaffected — it lives in `tools/` and
  doesn't touch `engine-script`.
- ADR-007 (owned scripting VM) is now load-bearing: the engine has a
  working interpreter. Phase 5 (rendering) starts with a real script
  runtime under it.
