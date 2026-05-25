# ADR-060 — sli aggregate opcodes

- Status: Accepted (opcodes + verifier + dispatcher + oracle landed in
  Phase-0 catchup PR alongside this ADR; codegen wiring is a follow-up)
- Date: 2026-05-24
- Phase: 0 (foundation closure; declared in the Phase-0 catchup PR,
  formalises a Phase-4 design commitment that was deferred for slicing)
- Companion: ADR-007 (owned scripting VM — the parent), ADR-035 (sli
  register VM + GC — the original opcode design that named these as
  reserved), ADR-059 (generational GC — supplies the write-barrier
  the mutating opcodes fire), ADR-013 (determinism contract — the
  oracle test pins cross-arch behaviour)

## Context

ADR-035 specified the sli VM as a 256-opcode register machine. The
Phase-4 implementation shipped 25 opcodes: control flow (Nop, Move,
ConstNil/True/False/Int/Float/Str), arithmetic, comparison, logical,
jumps, Call, FfiCall, Return, Trap. That covers numeric and string
inner-loop code but leaves four aggregate kinds — arrays, maps,
structs, closures — without opcodes. Codegen for the four AST
constructs that produce them (`ArrayLit`, `MapLit`, `StructLit`,
`Closure`, plus `Field` and `Index` accesses) emits `ConstNil`
placeholders annotated with a "PR 3 will replace this" comment.

The placeholders are a documented hole. The Phase-0 audit identified
them; this ADR closes the hole at the opcode layer.

The script-level types are already present (the typeck has `Array`,
`Map`, `Struct` types and the value tag has `Value::Array`,
`Value::Map`, `Value::Struct`, `Value::Closure`); the GC already
holds `Obj::Array`, `Obj::Map`, `Obj::Struct`, `Obj::Closure`. The
only missing piece is the dispatch path between them and the opcodes
that drive it.

## Decision

Twelve new opcodes land in the 0x70–0x7B range:

| byte | mnemonic       | layout                                              | notes                                  |
| ---- | -------------- | --------------------------------------------------- | -------------------------------------- |
| 0x70 | `ArrayNew`     | `dst:u8 n:u8 args:[u8; n]`                          | variable length; allocates `Obj::Array` |
| 0x71 | `ArrayGet`     | `dst:u8 arr:u8 idx:u8`                              | `idx` register holds `Int`             |
| 0x72 | `ArraySet`     | `arr:u8 idx:u8 src:u8`                              | fires write barrier                    |
| 0x73 | `ArrayLen`     | `dst:u8 arr:u8`                                     | result is `Int`                        |
| 0x74 | `MapNew`       | `dst:u8`                                            | allocates empty `Obj::Map`             |
| 0x75 | `MapGet`       | `dst:u8 map:u8 key:u8`                              | key register holds `Str`; miss → `Nil` |
| 0x76 | `MapSet`       | `map:u8 key:u8 src:u8`                              | fires write barrier                    |
| 0x77 | `StructNew`    | `dst:u8`                                            | allocates empty `Obj::Struct`          |
| 0x78 | `StructGet`    | `dst:u8 strct:u8 name_ki:u16le`                     | name is const-pool string              |
| 0x79 | `StructSet`    | `strct:u8 name_ki:u16le src:u8`                     | fires write barrier                    |
| 0x7A | `ClosureMake`  | `dst:u8 fn_idx:u16le n:u8 ups:[u8; n]`              | variable length; captures by value     |
| 0x7B | `CallClosure`  | `dst:u8 cls:u8 n:u8 args:[u8; n]`                   | variable length; upvalues at `r0..rk`  |

### Write-barrier discipline

Three mutating opcodes (`ArraySet`, `MapSet`, `StructSet`) fire the
write barrier (ADR-059 §4) when the new value is a GC handle. The
dispatcher path:

```rust
let target_handle = value_gc_handle(&new_val);
// ... mutate the heap ...
if let Some(target) = target_handle {
    heap.write_barrier(container, target);
}
```

`Heap::write_barrier` is itself O(1) and a no-op when the container is
in the same or younger generation than the target. The barrier fires
unconditionally for any value whose tag carries a `GcHandle`; the
heap-side check (old → young) is the cheap branch.

`ClosureMake` *also* captures GC handles via upvalues, but the
closure is freshly allocated (always nursery) so by construction no
old → young edge can be created on closure construction. The barrier
is therefore not needed; documented in the dispatcher's comment.

### Verifier

Every new opcode is rejected by the existing verifier's exhaustive
opcode match until the catchup PR extends it. The new arms check:

- Register operands within `max_register` bounds.
- `name_ki` const-pool index within `const_max` for `StructGet`/`StructSet`.
- `fn_idx` function-table index within `fn_max` for `ClosureMake`.

Runtime tag/key checks (e.g. "ArrayGet on a `Value::Map`",
"MapSet key not `Str`") are deferred to dispatch — they produce
`StopReason::Error` with a typed message. A future type-lattice
verifier (Phase 10+) would lift these to compile time; today they are
runtime trap conditions matching the existing arithmetic-error pattern.

### Codegen (deferred)

This ADR ships the opcodes + verifier + dispatcher + oracle test.
The codegen pass in `crates/engine-script/src/codegen.rs` still emits
`ConstNil` for `ArrayLit`, `MapLit`, `StructLit`, `Closure`, `Field`,
and `Index`. The wiring of codegen to the new opcodes is a follow-up
PR; the scope of this ADR is "the opcodes exist, are verifier-correct,
dispatcher-correct, write-barrier-correct, and round-trip-tested at
the bytecode level." Codegen flips placeholders to real emission
when ready; the existing oracle then becomes a regression net for
the codegen output.

## Rationale

Twelve opcodes is the minimum that closes the aggregate hole without
inviting scope creep. Three design choices:

1. **`MapNew` and `StructNew` are nullary** (no inline field-list).
   Codegen emits `New` followed by a sequence of `Set` opcodes. The
   alternative — variable-length `MapNew dst n (key val){n}` — adds
   one more variable-length opcode shape to the verifier and the
   `instr_len` table. The nullary form is simpler and the bytecode
   is the same size in practice.
2. **`ArrayNew` is variable-length** because array literals are
   commonly populated in one go (`[1, 2, 3]`); the alternative of
   `ArrayNew` + n `ArraySet`s is wasteful for the common case.
3. **`ClosureMake` and `CallClosure` are variable-length** for the
   same reason — closures capture a fixed number of upvalues known
   at lowering time, and the natural shape is "build with N
   captures."

The opcode bytes (0x70–0x7B) leave 0x7C–0x7F for future aggregate
extensions (set operations, generic iteration, etc.) without
disturbing the existing opcode space. 0x80–0xFE remains for the
255-opcode budget the spec allows.

The write-barrier discipline matches ADR-059's contract: the
dispatcher calls `Heap::write_barrier(source, target)` after the
mutation. The barrier itself decides if it fires (old → young
only). The dispatcher does not branch on generation; that's the
heap's job.

## Consequences

- `crates/engine-script/src/bytecode.rs` gains 12 opcode variants +
  their `from_u8` arms + their `instr_len` arms.
- `crates/engine-script/src/verify.rs` gains 5 verifier arms covering
  the 12 opcodes (grouped by operand shape).
- `crates/engine-script/src/vm/dispatch.rs` gains 12 dispatcher arms
  + two small helpers (`value_gc_handle` and `cls_upvalue_offset`).
- `crates/engine-script/tests/aggregate_ops_oracle.rs` (new, ~360 LOC)
  hand-assembles bytecode exercising each opcode. 10 tests cover:
  `ArrayNew`/`ArrayGet`/`ArrayLen` round-trip, `ArraySet` mutation,
  `MapNew`/`MapSet`/`MapGet` round-trip, `MapGet` missing-key,
  `StructNew`/`StructSet`/`StructGet` round-trip, `StructGet`
  missing-field, `ClosureMake` handle shape,
  `CallClosure` with captured upvalues, and a white-box write-barrier
  test that promotes an array to old gen and verifies the remembered
  set preserves a nursery target through one minor collection.
- Codegen wiring is the next deliverable. ADR-060 does not block it
  but does not include it.
- The `engine-script` crate's total test count: 84 (up from 74), all
  green.

## Risks and tradeoffs

- **No type-lattice verifier.** Runtime tag mismatches produce
  `StopReason::Error` rather than a compile-time error. Acceptable
  for Phase 0; a future verifier extension can lift many cases to
  compile time.
- **`MapSet`/`StructSet` use linear scans.** `Obj::Map`/`Obj::Struct`
  are flat `Vec<(Arc<str>, Value)>`. For small aggregate sizes
  (<32 entries) linear is faster than hashed; for larger, ADR-028's
  owned Robin Hood hash map will be the upgrade target once the GC
  exposes its allocator. Acceptable for Phase 0.
- **`ArrayLen` is not folded into bounds checks** at the verifier
  level. The verifier doesn't know array sizes statically; an
  out-of-bounds access is a runtime trap. Same as Lua/Python.
- **Closure upvalues are captured by value at `ClosureMake` time.**
  Mutation of the captured variable after closure creation is not
  reflected in the closure. This is "Rust-style closures with
  capture-by-move" semantics — matches the language ADR-034
  spec. A future `CaptureRef` opcode could add by-reference
  capture; not in scope.
- **The `ClosureMake` does no write barrier.** A future change that
  allowed `ClosureMake` to reach an existing old-gen closure object
  would need to revisit this. Today's implementation always
  allocates a fresh closure in the nursery; the comment in the
  dispatcher documents the invariant.

## Alternatives considered

- **Defer aggregates entirely to a "high-level" instruction set
  layered above the VM.** Doubles the dispatch indirection; loses
  determinism. Rejected.
- **One opcode per aggregate kind that takes a "subop" byte.** Saves
  opcode space; loses the verifier's static dispatch. Rejected.
- **Inline-populated `MapNew` and `StructNew`.** Adds two more
  variable-length opcodes for marginal byte savings. Rejected per
  rationale above.
- **`StructGet`/`StructSet` keyed by register-Str instead of
  const-pool index.** Loses one byte of static guarantee (the
  verifier knows the name at verify time, not run time). Rejected.
- **Closure capture-by-ref via an upvalue cell.** Considered;
  cell-based capture requires another aggregate kind and a third
  reference type. Deferred to future ADR if real workloads need it.

## Verification

- `cargo test -p engine-script --test aggregate_ops_oracle` — 10
  hand-assembled tests covering each opcode at the bytecode level.
- `cargo test -p engine-script` — full suite, 84 tests, green
  (the 10 new tests + 74 pre-existing).
- The verifier's own tests in `tests/verifier.rs` (10 tests)
  continue to pass — the verifier additions are additive.
- The vm oracle (`tests/vm_oracle.rs`) and its committed BLAKE3
  golden continue to pass; the new opcodes don't appear in the
  compiled corpus because codegen is unchanged.
- The GC oracle (`tests/gc_oracle.rs`, 5 tests) and the pause
  oracle (`tests/gc_pause_oracle.rs`, 1 test) continue to pass —
  the GC's generational refactor is consistent with the new opcodes.
- The dispatch loop's write-barrier wiring is end-to-end tested by
  the `write_barrier_records_old_to_young_via_array_set` test in
  the new oracle.
- Future codegen wiring will land its own oracle (a compile-and-run
  test for each aggregate-using AST construct) per the precedent
  set by `tests/vm_oracle.rs`.
