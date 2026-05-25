# engine-script

The `sli` language toolchain: compiler front-end, register VM,
generational GC, bytecode verifier, hot-reload, debugger
protocol, REPL (spec §IV.7, §XII; ADRs 007, 034, 035, 036, 059,
060).

## Purpose

The engine's owned scripting subsystem. Where game logic lives,
where the engine's determinism contract extends from the
simulation thread into user-authored code, where hot-reload and
the debugger anchor.

The Phase 4 thrust delivered the language end-to-end (ADRs
034–037); the Phase-0 catchup PR (ADRs 059 + 060) closes the
remaining gaps — generational GC and aggregate/closure opcodes.

## Modules

| Module                     | Contents |
| -------------------------- | -------- |
| `source` / `lex`           | Source loader + Unicode-aware lexer. Lossless token stream feeds the parser. |
| `parse` / `ast`            | Pratt-style recursive-descent parser producing the AST. Aggregate literals (`[...]`, `{...}`, struct literal, closure `\|x\| x + 1`) added by ADR-060. |
| `resolve` / `typeck`       | Name resolution and the bidirectional Hindley-Milner-flavoured type checker (ADR-034). |
| `ir`                       | SSA-like IR; AST → IR lowering; the layer where aggregate access becomes the `Get/Set/Push` family. |
| `consteval`                | Compile-time constant folding (ADR-034). |
| `codegen`                  | IR → bytecode. Emits the 256-opcode register VM stream. |
| `bytecode`                 | Bytecode definitions: opcode table, operand encoding, function table, constant pool. ADR-060 adds 12 aggregate/closure opcodes (0x70–0x7B). |
| `verify`                   | Bytecode verifier: register-bounds, type-flow, control-flow safety (ADR-035). Aggregate opcodes carry per-opcode rules added by ADR-060. |
| `vm::dispatch`             | The hot interpreter loop. Direct-threaded dispatch per opcode. Write-barrier hook (`gc::barrier`) calls into the GC on cross-generation stores. |
| `vm::frame` / `vm::value`  | Frame layout + `Value` representation (NaN-boxed; spec §IV.7). |
| `gc::nursery`              | Young-generation bump-allocator (ADR-059). Objects promoted to old gen on second survival. |
| `gc::old_gen`              | Mark-and-sweep old generation with free list (ADR-059). |
| `gc::remembered`           | Card-marking remembered set; one byte per 512-byte old-gen region (ADR-059). |
| `gc::barrier`              | Dijkstra-style generational write barrier; called from `vm::dispatch` on `Store/MapSet/StructSet/ArraySet` when source is old-gen and target is young (ADR-059). |
| `gc::mod`                  | `Heap` façade unifying nursery + old gen + remembered set; major/minor GC paths. |
| `breakpoints_toml`         | Breakpoint persistence to `.engine/debug/breakpoints.toml`. (Acknowledged spec deviation per ADR-051 — spec wanted RON.) |
| `debug_proto`              | Owned binary debugger protocol (ADR-036). |
| `debug`                    | Debugger runtime: breakpoint dispatch, single-step, frame introspection. |
| `watch_expr`               | Watch-expression evaluator (sandboxed; cannot mutate live program state). |
| `reload`                   | Function-level hot-reload (function-table swap; ADR-036). |
| `repl`                     | Interactive REPL attached to live engine processes. |
| `ffi`                      | Foreign-function boundary to Rust host code. |
| `asset`                    | Bytecode pak format and loader (content-addressed per ADR-008). |
| `diag` / `ext`             | Diagnostics formatting + extension hooks. |
| `lib`                      | Crate entry: re-exports + top-level `Compiler`, `Vm`, `Heap`. |

## Determinism

- The VM is strict-IEEE-754 throughout (ADR-013, ADR-027). No
  FMA on script arithmetic; bit-equal results across
  architectures.
- Hash-table iteration in script (`for k, v in map { ... }`)
  iterates in deterministic insertion+hash order (using the
  engine's owned hash map per ADR-028, not `std::HashMap`).
- BLAKE3 RNG bindings (`engine_core::rng::Rng`, ADR-057) are
  exposed to script through `ffi`; script code can draw
  deterministic random values keyed by the same
  `(seed, frame, channel)` discipline as Rust code.
- GC pauses are bounded; the Phase-0 catchup generational GC
  (ADR-059) targets p99 ≤ 250 µs.
- Function-table swap during hot-reload (ADR-036) preserves
  determinism — replayed input under the new function table
  produces the new function's deterministic output.

## Oracles

- `tests/parser.rs` — parser corpus (round-trip + golden).
- `tests/typeck.rs` — type inference + diagnostic golden.
- `tests/ir.rs` — IR lowering golden.
- `tests/codegen_no_trap.rs` — bytecode generation never emits
  opcode `0xFF` (TRAP); structural invariant.
- `tests/verifier.rs` — verifier corpus (well-formed bytecode
  passes; ill-formed bytecode is rejected with the expected
  typed error).
- `tests/vm_oracle.rs` — VM execution corpus (round-trip + golden).
- `tests/compile_parity.rs` — front-end-to-back-end determinism:
  same source produces same bytecode across runs.
- `tests/gc_oracle.rs` — GC functional correctness (allocation,
  collection, no-double-free, weak refs).
- `tests/gc_pause_oracle.rs` — GC pause-time bound. Promoted to
  a hard CI gate by ADR-059's Phase-0 catchup PR. Target:
  p99 ≤ 250 µs.
- `tests/aggregate_ops_oracle.rs` (added by ADR-060) — round-trip
  test for every new aggregate/closure opcode.
- `tests/hot_reload.rs` — function-table swap correctness.
- `tests/debug_protocol.rs` — debugger wire protocol round-trip.
- `tests/breakpoint_persistence.rs` — TOML breakpoint
  persistence (ADR-051 acknowledged deviation).
- `tests/watch_expr_safety.rs` — watch expression sandbox
  cannot mutate live state.
- `tests/ffi.rs` — Rust↔script FFI boundary.

Golden files live in `tests/goldens/`:
`sli-parse.golden`, `sli-typeck.golden`, `sli-ir.golden`,
`sli-compile.golden`, `sli-vm.golden`.

Cross-architecture determinism: the front-end goldens and the
VM goldens are byte-identical on x86-64 and aarch64 in CI.

## Dependencies

`engine-core` (RNG, ECS hooks, telemetry), `engine-math` (strict
math), `engine-reflect` (script ↔ component reflection),
`engine-platform` (filesystem, time), `engine-ecs-macro`
(derives) — all Level 0 / 1 crates. No third-party deps in
the foundation tier.
