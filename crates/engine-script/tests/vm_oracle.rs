//! End-to-end VM oracle (PR 2, ADR-035).
//!
//! Compiles a hand-picked subset of the PR-1 corpus, verifies the
//! bytecode, and executes each program in the VM, asserting the
//! expected return value. Takes a BLAKE3 digest over `(name, result)`
//! pairs and compares against a committed golden so cross-arch
//! determinism rides on the same pattern as PR-1's `compile_parity`.

use blake3::Hasher;
use engine_script::{Compiler, Source, SourceMap, StopReason, Value, Vm, verify};
use std::fs;
use std::path::PathBuf;

struct Case {
    name: &'static str,
    source: &'static str,
    entry: &'static str,
    args: Vec<Value>,
    expected: Value,
}

fn cases() -> Vec<Case> {
    vec![
        Case {
            name: "arith_int_pure",
            source: r#"
fn main() -> i64 { return 1 + 2 * 3 - 4 / 2 + 5 % 3; }
"#,
            entry: "main",
            args: vec![],
            // 1 + 6 - 2 + 2 = 7
            expected: Value::Int(7),
        },
        Case {
            name: "arith_float_pure",
            source: r#"
fn main() -> f64 {
    let x: f64 = 3.5;
    let y: f64 = 1.5;
    return x * y - 0.25;
}
"#,
            entry: "main",
            args: vec![],
            expected: Value::Float(3.5 * 1.5 - 0.25),
        },
        Case {
            name: "while_sum_to_10",
            source: r#"
fn sum_to(n: i64) -> i64 {
    let mut i: i64 = 0;
    let mut acc: i64 = 0;
    while i < n {
        acc = acc + i;
        i = i + 1;
    }
    return acc;
}
"#,
            entry: "sum_to",
            args: vec![Value::Int(10)],
            expected: Value::Int(45),
        },
        Case {
            name: "recursive_fib_10",
            source: r#"
fn fib(n: i64) -> i64 {
    if n < 2 { return n; }
    return fib(n - 1) + fib(n - 2);
}
"#,
            entry: "fib",
            args: vec![Value::Int(10)],
            expected: Value::Int(55),
        },
        Case {
            name: "if_classify_minus_one",
            source: r#"
fn classify(n: i64) -> i64 {
    if n < 0 {
        return -1;
    } else {
        if n == 0 {
            return 0;
        } else {
            return 1;
        }
    }
}
"#,
            entry: "classify",
            args: vec![Value::Int(-42)],
            expected: Value::Int(-1),
        },
        Case {
            name: "if_classify_zero",
            source: r#"
fn classify(n: i64) -> i64 {
    if n < 0 { return -1; } else { if n == 0 { return 0; } else { return 1; } }
}
"#,
            entry: "classify",
            args: vec![Value::Int(0)],
            expected: Value::Int(0),
        },
        Case {
            name: "if_classify_pos",
            source: r#"
fn classify(n: i64) -> i64 {
    if n < 0 { return -1; } else { if n == 0 { return 0; } else { return 1; } }
}
"#,
            entry: "classify",
            args: vec![Value::Int(7)],
            expected: Value::Int(1),
        },
        Case {
            name: "logic_xor_true_false",
            source: r#"
fn xor(a: bool, b: bool) -> bool {
    return (a || b) && !(a && b);
}
"#,
            entry: "xor",
            args: vec![Value::Bool(true), Value::Bool(false)],
            expected: Value::Bool(true),
        },
        Case {
            name: "logic_xor_true_true",
            source: r#"
fn xor(a: bool, b: bool) -> bool {
    return (a || b) && !(a && b);
}
"#,
            entry: "xor",
            args: vec![Value::Bool(true), Value::Bool(true)],
            expected: Value::Bool(false),
        },
        Case {
            name: "clip_in_range",
            source: r#"
fn clip(x: i64, lo: i64, hi: i64) -> i64 {
    if x < lo { return lo; }
    if x > hi { return hi; }
    return x;
}
"#,
            entry: "clip",
            args: vec![Value::Int(5), Value::Int(0), Value::Int(10)],
            expected: Value::Int(5),
        },
        Case {
            name: "clip_clamps_low",
            source: r#"
fn clip(x: i64, lo: i64, hi: i64) -> i64 {
    if x < lo { return lo; }
    if x > hi { return hi; }
    return x;
}
"#,
            entry: "clip",
            args: vec![Value::Int(-3), Value::Int(0), Value::Int(10)],
            expected: Value::Int(0),
        },
    ]
}

fn run_case(case: &Case) -> Value {
    let mut sm = SourceMap::new();
    let id = sm.add(Source::new(format!("{}.sli", case.name), case.source));
    let compiled = Compiler::new().compile(id, sm.get(id)).unwrap();
    assert!(
        !compiled.diagnostics.has_errors(),
        "{} produced compile diagnostics: {:?}",
        case.name,
        compiled
            .diagnostics
            .all()
            .iter()
            .map(|d| &d.message)
            .collect::<Vec<_>>()
    );
    verify(&compiled.bytecode).unwrap_or_else(|e| panic!("{}: verify failed: {e}", case.name));
    let mut vm = Vm::new(compiled.bytecode);
    match vm.call(case.entry, case.args.clone()) {
        StopReason::Returned(v) => v,
        other => panic!("{} stopped unexpectedly: {other:?}", case.name),
    }
}

#[test]
fn vm_returns_expected() {
    for case in cases() {
        let got = run_case(&case);
        assert_eq!(got, case.expected, "{} produced wrong result", case.name);
    }
}

#[test]
fn vm_oracle_digest_matches_golden() {
    let mut hasher = Hasher::new();
    for case in cases() {
        let v = run_case(&case);
        hasher.update(case.name.as_bytes());
        hasher.update(b" => ");
        hasher.update(format!("{v:?}").as_bytes());
        hasher.update(b"\n");
    }
    let digest = hasher.finalize().to_hex().to_string();

    let golden_path: PathBuf = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("goldens")
        .join("sli-vm.golden");

    if std::env::var("ENGINE_GOLDEN_WRITE").is_ok() {
        fs::write(&golden_path, format!("{digest}\n")).unwrap();
        eprintln!("wrote {}", golden_path.display());
        return;
    }

    let expected = fs::read_to_string(&golden_path)
        .expect("missing golden — regenerate with ENGINE_GOLDEN_WRITE=1");
    assert_eq!(
        digest,
        expected.trim(),
        "sli-vm digest drift\n  expected: {expected}\n  got     : {digest}",
    );
}
