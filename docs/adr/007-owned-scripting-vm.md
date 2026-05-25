# ADR-007 — Owned scripting VM (sli)

- Status: Accepted
- Date: 2026-05-18 (expanded 2026-05-24 per audit §15 Phase-0 ADR sweep)
- Phase: 0 (pre-v1.0 — contract-exempt per risk R-03)
- Companion: ADR-034 (sli front-end), ADR-035 (sli register VM + GC),
  ADR-036 (sli hot-reload + debugger + REPL), ADR-059 (generational
  GC — Phase 0 catchup), ADR-060 (sli aggregates and closures — Phase
  0 catchup), ADR-013 (determinism contract)

## Context

The engine needs a scripting language for game logic. The portfolio
target — 1M entities, deterministic frame digests, sub-millisecond
GC pauses on the simulation thread — drives the choice.

Three architectural options were considered:

- **Embed Lua** (5.4 or LuaJIT). Tiny VM, fast, mature debugger
  ecosystem. Garbage collector is a known stutter source; the
  GC pause is observable on the simulation thread because Lua
  has no parallel-GC mode. Determinism across hardware is not
  a Lua guarantee (the VM uses `double` arithmetic; FP rounding
  differences are visible).
- **Embed Python.** Out of consideration for the same reasons
  most engines reject Python: GIL-bound, slow, GC behavior
  unpredictable.
- **Embed WASM** (Wasmtime / Wasmer). Memory-safe; cross-language;
  no native debugger workflow; module-instance lifecycle does
  not match hot-reload semantics the spec wants.
- **Own the VM.** Crafting Interpreters / Dragon Book lineage:
  a small register VM with a verifier-backed bytecode, an owned
  generational GC, an owned debugger protocol, and determinism
  guarantees built into the bytecode semantics from day one.

The spec (§IV.7) is explicit: scripting determinism is a first-
class contract, the script GC pause must fit inside the frame
budget, and hot-reload must work without restarting the engine.
None of the embed options simultaneously satisfy all three.

## Decision

The engine owns its scripting VM: `sli` (engine-script crate
family). The contract:

- Source language: a Rust-flavoured statically typed scripting
  language (parser/typeck/IR — ADR-034).
- Bytecode: a 256-opcode register VM with a verifier (ADR-035).
- Memory: a tri-color generational GC with sub-millisecond pause
  targets (ADR-059 fully realises the generational design after
  the Phase-0 catchup PR).
- Determinism: strict IEEE-754 arithmetic on the VM (ADR-013);
  the VM's frame execution produces byte-identical world state
  across architectures.
- Hot-reload: function-level reload while the engine is running
  (ADR-036).
- Debugger: an owned binary protocol over IPC, with breakpoint
  persistence to disk (ADR-036 — currently TOML per the ADR-051
  acknowledged deviation; spec called for RON).
- REPL: an interactive shell that can attach to a running engine
  process (ADR-036).

The choice maps to spec §IV.7's "owned scripting VM" requirement
and to the foundation-layer R-02 stance ("own the layer to own
the bugs and the determinism").

## Rationale

Three properties an owned VM can guarantee that an embed cannot:

1. **Determinism by construction.** Every opcode's IEEE-754
   semantics are fixed by the engine. No `--ffloat-store`
   surprises, no JIT-emitted code that differs per CPU
   generation, no language-level non-determinism (hash-table
   iteration order is fixed by the engine, not by the host
   stdlib).
2. **GC pause budget.** A generational GC the engine owns can be
   tuned to the frame budget (ADR-059's p99 ≤ 250 µs target).
   An embedded GC the engine cannot tune is a perpetual
   debugging cost; LuaJIT's GC pause is the standard cited
   example.
3. **Hot-reload + debugger integration.** The bytecode format
   and the debugger protocol are the engine's; reload semantics
   (function-table swap, live-stack adaptation) are designed in
   from PR 1. Embedded VMs require workarounds (Lua's
   `package.loaded[...] = nil` trick is not a real hot-reload).

The Crafting Interpreters / Dragon Book lineage means the
implementation pattern is well-mapped: register VMs are simpler
than stack VMs to debug, the tri-color generational GC is a
literature-staple algorithm, and the bytecode verifier (ADR-035)
is the analog of WebAssembly's verifier — a well-understood
discipline.

## Consequences

- The engine ships its own language. Contributors learn `sli`;
  `docs/architecture/engine-script.md` (Phase-0 catchup) is the
  entry point.
- Phase 4 delivered the VM end-to-end (ADRs 034–036). Phase-0
  catchup (after the audit) closes the remaining gaps:
  generational GC (ADR-059) and aggregate/closure opcodes
  (ADR-060).
- An owned debugger protocol means the editor's debugger UI
  (Phase 10) speaks the engine's own wire format; no external
  debugger compatibility is needed.
- The reference library / training cost for `sli` is small (the
  language is intentionally narrow), but it exists.

## Risks and tradeoffs

- **No external sli IDE in 2026.** Mitigation: `engine-tui`
  (Phase 10) plus the REPL cover the inner-loop case; the
  editor (Phase 10) embeds the debugger UI.
- **Owning a language is expensive.** Mitigation: scope is
  deliberately narrow — `sli` is not Python or Lua, it is a
  domain-specific language for game scripting; the reference
  surface is bounded.
- **Performance vs. JIT.** `sli` is an interpreter, not a JIT;
  hot code paths in scripting will be slower than native
  Rust. Pattern: scripting is for logic, not for the
  inner loop. Performance-sensitive code lives in Rust systems.
- **Determinism contract violations** in `sli` would silently
  break netcode replay. Mitigation: the determinism oracle
  (ADR-013) runs `sli` workloads across architectures.

## Alternatives considered

- **Lua / LuaJIT.** Mature; not deterministic; GC-pause
  problematic. Rejected.
- **WASM embedding.** Module lifecycle wrong for hot-reload;
  no debugger workflow. Rejected.
- **Python.** Performance unsuitable. Rejected.
- **GDScript-style approach** (a language tied to one engine's
  conventions). Identical to what we ended up with; the
  difference is which language and which authoring conventions.
- **A subset of Rust as the scripting language.** Considered;
  rejected because Rust's complexity (lifetimes, generics,
  trait coherence) does not match scripting's ergonomic goals.

## Verification

- Phase 4 closure: `cargo test -p engine-script` green across
  parser, typeck, IR, VM, verifier, GC, debugger.
- Cross-architecture determinism: `engine-script` is in the
  CI determinism job's scope; sli frame digests are
  byte-identical on x86-64 and aarch64.
- GC pause target: `crates/engine-script/tests/gc_pause_oracle.rs`
  becomes a hard CI gate after the Phase-0 catchup PR (ADR-059).
- Aggregate opcode round-trip: `tests/aggregate_ops_oracle.rs`
  (added by ADR-060 work) covers every new opcode.
- Hot-reload: the function-level reload integration test in
  Phase 4 PR 3 verifies live function-table swap; ADR-036's
  test corpus.
- Debugger protocol: the protocol's reproducibility test ensures
  client-server compatibility across builds.
