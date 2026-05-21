//! FFI round-trip oracle (PR 2, ADR-035).
//!
//! Registers a Rust callback with the VM's `CallTable`, calls it from
//! script-emulated bytecode, and asserts marshalling round-trips every
//! `Value` variant the PR-2 wire supports (primitives + `Arc<str>`;
//! GC-backed variants are exercised by `tests/gc_oracle.rs`).

use engine_script::ffi::CallTable;
use engine_script::gc::Heap;
use engine_script::vm::Value;
use std::sync::Arc;

fn echo(args: &[Value], _heap: &mut Heap) -> Result<Value, String> {
    args.first()
        .cloned()
        .ok_or_else(|| "expected one argument".into())
}

fn add(args: &[Value], _heap: &mut Heap) -> Result<Value, String> {
    match (args.first(), args.get(1)) {
        (Some(Value::Int(a)), Some(Value::Int(b))) => Ok(Value::Int(a + b)),
        _ => Err("expected (Int, Int)".into()),
    }
}

#[test]
fn registers_and_invokes() {
    let mut table = CallTable::new();
    let id = table.register("echo", Some(1), echo);
    assert_eq!(table.id_of("echo"), Some(id));
    let mut heap = Heap::with_default_config();
    let out = table.call(id, &[Value::Int(7)], &mut heap).unwrap();
    assert_eq!(out, Value::Int(7));
}

#[test]
fn arity_mismatch_is_rejected() {
    let mut table = CallTable::new();
    let id = table.register("add", Some(2), add);
    let mut heap = Heap::with_default_config();
    let err = table.call(id, &[Value::Int(1)], &mut heap).unwrap_err();
    assert!(err.contains("expected 2 args"));
}

#[test]
fn roundtrips_every_primitive_value() {
    let mut table = CallTable::new();
    let id = table.register("echo", Some(1), echo);
    let mut heap = Heap::with_default_config();
    let inputs = vec![
        Value::Nil,
        Value::Bool(true),
        Value::Bool(false),
        Value::Int(i64::MIN),
        Value::Int(0),
        Value::Int(i64::MAX),
        Value::Float(0.0),
        Value::Float(-0.0),
        Value::Float(f64::INFINITY),
        Value::Str(Arc::from("sliced engine")),
        Value::Entity(42),
        Value::Handle(0xDEAD_BEEF),
    ];
    for v in inputs {
        let out = table.call(id, std::slice::from_ref(&v), &mut heap).unwrap();
        assert_eq!(out, v, "ffi did not round-trip {v:?}");
    }
}
