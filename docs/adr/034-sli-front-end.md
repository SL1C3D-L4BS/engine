# ADR-034 · sli front-end: lex / parse / typeck / SSA IR

- Status: Accepted
- Date: 2026-05-20
- Phase: 4 (SCRIPTING — spec Part XXI, PR 1)

## Context

Phase 4 ships the scripting layer end-to-end. PR 1 owns the compiler
front-end: source text in, optimised SSA IR out, with diagnostics
collected along the way. The on-disk representation (bytecode), the
runtime (register VM + GC), and the developer surfaces (hot-reload,
debugger, REPL) land in PRs 2–3 on top of this surface; the Slang
toolchain in PR 4 is independent.

Spec V.1–V.4 names sli (`.sli`, with `.bp` as a one-major-version
legacy alias) as the engine's authored scripting language. ADR-007 set
the policy: an owned VM, no Lua / Python / WASM embed, Crafting
Interpreters + Dragon Book lineage. ADR-034 is the implementation
record for the front-end half of that policy.

## Decision

A hand-written lexer feeds a Pratt parser into a typed AST. Name
resolution and a bottom-up type checker mutate the AST in place; a
const-expression evaluator reduces `const` initialisers; the AST then
lowers to a register-based SSA IR. Three classical scalar passes
(constant folding, common-subexpression elimination, dead-code
elimination) iterate to a fixed point.

The front-end has **no third-party parser-generator, regex-engine, or
vendored interpreter as a dependency**. The CI grep guard added with
this PR rejects `lalrpop`, `pest`, `nom`, `combine`, `chumsky`, `rlua`,
`mlua`, `wasmtime`, `wasmer`, `cranelift`, and `inkwell` anywhere
under `crates/engine-script/`. The only added dependencies are
`engine-core` (for the owned `HashMap`, ADR-028), `engine-platform`
(arena scratch, ADR-026), `engine-reflect` (ECS type registry — used
by the PR-2 FFI), and `blake3` (dev-only, for the cross-arch golden
digest below).

Module layout (`crates/engine-script/src/`):

- `source.rs` — `Source`, `SourceMap`, `Span` (`file_id` + half-open
  byte range).
- `diag.rs` — `Diagnostic`, `Severity`, accumulating `Diagnostics`.
- `lex.rs` — hand-written tokenizer; `.sli` source text → `Vec<Token>`.
- `parse.rs` — Pratt parser to `ast::Module`; pretty-printer for
  the round-trip oracle.
- `ast.rs` — typed AST (`Expr`, `Stmt`, `Decl`, `Type`).
- `resolve.rs` — single-pass scope tree + global symbol table.
- `typeck.rs` — bottom-up checker; numeric-literal hints flow through
  `let` annotations, function-argument types, struct-field types, and
  `return`-statement targets.
- `ir.rs` — SSA IR, lowering, const-fold / CSE / DCE passes, and a
  deterministic text serialiser (the format the golden digest covers).
- `consteval.rs` — pure `const` expression evaluator (spec IV.7).
- `ext.rs` — file-extension routing: `.sli` canonical, `.bp` alias.

Public surface (`lib.rs`): `Compiler`, `Compiled`, `CompileError`,
`DebugInfo`, plus re-exports for `Module`, `Source`, `SourceMap`,
`Span`, `Diagnostic`, `Diagnostics`, `Severity`, `IrModule`.

## Cross-architecture determinism

`tests/compile_parity.rs` compiles every fixture in the corpus, takes
a BLAKE3 digest over the concatenated optimised-IR serialisations, and
compares against `tests/goldens/sli-compile.golden`. The Phase-3
determinism matrix runs the test on x86-64 and aarch64; two
architectures agreeing with one digest proves cross-arch
byte-equality transitively (ADR-013 pattern, same as engine-math and
engine-core).

The IR text format is line-oriented (`function/index: instruction` per
line); float constants serialise via `to_bits` so denormals and
signed-zero choices are byte-stable. The serialiser is owned in
`ir.rs::serialise` for that reason — `Debug` would suffice today, but
keeping the format separate from a derived trait means a future
`#[derive(Debug)]` tweak cannot silently drift the golden.

## Test surface (this PR)

- `tests/compile_parity.rs` — committed BLAKE3 golden over the corpus.
- `tests/parser.rs` — `print(parse(s))` idempotence over the corpus.
- `tests/typeck.rs` — positive (every fixture cleanly checks) and
  negative (mismatch, undefined name, missing field, arity, condition
  type) cases.
- `tests/ir.rs` — hand-built `IrFn` fixtures exercise const-fold,
  CSE, DCE, and the optimise() fixpoint independently.

The corpus is 20 sorted fixtures exercising arithmetic (int + float),
control flow (`if` / `else` / `while`), structs (declaration,
literal, nested field access), closures, ECS query sugar
(`Query<T>`, `Res<T>`, `ResMut<T>`, `Entity`), and a mixed program
that stitches several of those together.

## Consequences

- Phase 4 PR 2 (`bytecode.rs` / `verify.rs` / `vm/`) consumes
  `Compiled::ir` directly — no further intermediate format.
- Phase 4 PR 3 (`reload.rs`) reuses the same `Compiler` to re-compile
  on every `WatchEvent::Modified`; `DebugInfo` gives the debugger
  enough span data to re-arm line breakpoints after a swap.
- Phase 5+ (rendering) consumes script modules through the Asset
  surface PR 2 adds (`impl Asset for ScriptModule`).
- An owned parser generator stays an option for later if grammar
  complexity grows past hand-written ergonomics; ADR-034 explicitly
  rejects pulling one in *speculatively*, in line with the
  foundation-layer R-02 norm.
