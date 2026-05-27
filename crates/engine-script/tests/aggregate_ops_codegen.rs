//! End-to-end codegen oracle for ADR-060 aggregate opcodes.
//!
//! `aggregate_ops_oracle.rs` exercises the dispatcher + verifier from
//! hand-assembled bytecode. This oracle exercises the *codegen* path:
//! sli source → lex → parse → typeck → codegen → verifier → VM →
//! `Value`. Each test names the AST shape it targets and checks that
//! the runtime answer is what a hand-evaluator would produce.
//!
//! The corpus is intentionally small (~one test per AST shape) — broad
//! parity coverage is the compile-parity oracle's job. The point here
//! is "did codegen wire the bytecode correctly" rather than "is the
//! compile output stable."
//!
//! See ADR-060 §Codegen for the AST→opcode mapping.
//!
//! - `ExprKind::ArrayLit` → `ArrayNew`
//! - `ExprKind::Index` (on `Type::Array`) → `ArrayGet`
//! - `ExprKind::Index` (on `Type::Map`) → `MapGet`
//! - `ExprKind::MapLit` → `MapNew` + per-pair `MapSet`
//! - `ExprKind::StructLit` → `StructNew` + per-field `StructSet`
//! - `ExprKind::Field` → `StructGet`
//! - `ExprKind::Closure` → `ClosureMake` (capture list discovered by
//!   walking free variables in the closure body)

use engine_script::bytecode::Module;
use engine_script::ffi::CallTable;
use engine_script::gc::Heap;
use engine_script::vm::{CallFrame, Value, dispatch, run};
use engine_script::{Compiler, Source, SourceMap, verify};

fn compile_source(name: &str, src: &str) -> Module {
    let mut sm = SourceMap::new();
    let id = sm.add(Source::new(name, src));
    let compiled = Compiler::new()
        .compile(id, sm.get(id))
        .expect("compilation must succeed for this fixture");
    assert!(
        !compiled.diagnostics.has_errors(),
        "{}: unexpected diagnostics: {:?}",
        name,
        compiled
            .diagnostics
            .all()
            .iter()
            .map(|d| &d.message)
            .collect::<Vec<_>>(),
    );
    compiled.bytecode
}

fn run_main(module: Module) -> Value {
    verify::verify(&module).expect("verifier accepts codegen output");
    // Locate `main` (every fixture defines a no-arg `main`).
    let main_id = module
        .function_id("main")
        .expect("fixture must define a `main` function");
    let f = &module.functions[main_id as usize];
    let frame = CallFrame::new(main_id, f.max_register, None);
    let mut stack = vec![frame];
    let mut heap = Heap::with_default_config();
    let ffi = CallTable::new();
    match run(&module, &mut stack, &mut heap, &ffi) {
        dispatch::StopReason::Returned(v) => v,
        other => panic!("expected Returned from main, got {other:?}"),
    }
}

#[test]
fn array_literal_indexed_read() {
    // ArrayLit + ArrayGet via codegen.
    let m = compile_source(
        "array_index.sli",
        r#"
fn main() -> i64 {
    let xs: Array<i64> = [10, 20, 30];
    return xs[1];
}
"#,
    );
    assert_eq!(run_main(m), Value::Int(20));
}

#[test]
fn array_literal_full_sum() {
    let m = compile_source(
        "array_sum.sli",
        r#"
fn main() -> i64 {
    let xs: Array<i64> = [10, 20, 30];
    return xs[0] + xs[1] + xs[2];
}
"#,
    );
    assert_eq!(run_main(m), Value::Int(60));
}

#[test]
fn map_literal_lookup_returns_value() {
    // MapLit + MapGet via codegen.
    let m = compile_source(
        "map_get.sli",
        r#"
fn main() -> i64 {
    let m: Map<str, i64> = ["a" => 1, "b" => 2, "c" => 3];
    return m["b"];
}
"#,
    );
    assert_eq!(run_main(m), Value::Int(2));
}

#[test]
fn map_literal_combined_arithmetic() {
    let m = compile_source(
        "map_arith.sli",
        r#"
fn main() -> i64 {
    let m: Map<str, i64> = ["x" => 100, "y" => 25];
    return m["x"] - m["y"];
}
"#,
    );
    assert_eq!(run_main(m), Value::Int(75));
}

#[test]
fn struct_literal_field_read() {
    // StructLit + Field via codegen.
    let m = compile_source(
        "struct_field.sli",
        r#"
struct Point { x: i64, y: i64 }

fn main() -> i64 {
    let p: Point = Point { x: 3, y: 4 };
    return p.x + p.y;
}
"#,
    );
    assert_eq!(run_main(m), Value::Int(7));
}

#[test]
fn struct_literal_with_two_fields_returns_via_field() {
    let m = compile_source(
        "struct_pick.sli",
        r#"
struct Counter { value: i64, step: i64 }

fn main() -> i64 {
    let c: Counter = Counter { value: 42, step: 1 };
    return c.value;
}
"#,
    );
    assert_eq!(run_main(m), Value::Int(42));
}

#[test]
fn nested_struct_field_chain() {
    // `o.i.v` chains StructGet → StructGet.
    let m = compile_source(
        "nested.sli",
        r#"
struct Inner { v: i64 }
struct Outer { i: Inner, k: i64 }

fn main() -> i64 {
    let i: Inner = Inner { v: 7 };
    let o: Outer = Outer { i: i, k: 11 };
    return o.i.v + o.k;
}
"#,
    );
    assert_eq!(run_main(m), Value::Int(18));
}

#[test]
fn closure_with_capture_via_callclosure() {
    // ClosureMake + CallClosure via codegen — the captured `k` is
    // resolved from the enclosing scope and packed into the closure's
    // capture list.
    let m = compile_source(
        "closure_add.sli",
        r#"
fn main() -> i64 {
    let k: i64 = 10;
    let add_k = |x: i64| x + k;
    return add_k(5);
}
"#,
    );
    assert_eq!(run_main(m), Value::Int(15));
}

#[test]
fn closure_with_two_captures_chains_addition() {
    let m = compile_source(
        "closure_two.sli",
        r#"
fn main() -> i64 {
    let a: i64 = 3;
    let b: i64 = 4;
    let combine = |x: i64| a + b + x;
    return combine(5);
}
"#,
    );
    assert_eq!(run_main(m), Value::Int(12));
}

#[test]
fn closure_without_captures_still_callable() {
    // No free variables — captures list is empty; bytecode emits
    // `ClosureMake dst fn_idx 0` followed by `CallClosure dst 1 arg`.
    let m = compile_source(
        "closure_pure.sli",
        r#"
fn main() -> i64 {
    let f = |x: i64| x * x;
    return f(6);
}
"#,
    );
    assert_eq!(run_main(m), Value::Int(36));
}
