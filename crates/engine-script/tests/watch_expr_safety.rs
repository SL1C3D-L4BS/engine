//! Watch-expression purity oracle (PR 3, ADR-036).
//!
//! The verifier rejects every form that could observably mutate state
//! or call non-pure code. PR 3 ships an empty `PURE_BUILTINS`
//! allowlist; once stdlib pure builtins (`Math.*`, `Str.*`) land, they
//! enter that list.

use engine_script::watch_expr::{WatchError, validate};

#[test]
fn pure_expression_accepted() {
    assert!(validate("1 + 2").is_ok());
    assert!(validate("x + 1").is_ok());
    assert!(validate("a.b").is_ok());
    assert!(validate("(x > 0) && (y < 10)").is_ok());
}

#[test]
fn function_call_rejected() {
    let err = validate("foo(1)").unwrap_err();
    assert!(matches!(err, WatchError::Impure { .. }), "{err:?}");
}

#[test]
fn ffi_call_rejected() {
    // The validator treats any callsite as impure unless the callee is
    // in PURE_BUILTINS (which is empty in PR 3).
    let err = validate("print(\"hi\")").unwrap_err();
    assert!(matches!(err, WatchError::Impure { .. }), "{err:?}");
}

#[test]
fn closure_rejected() {
    let err = validate("|x| x + 1").unwrap_err();
    assert!(matches!(err, WatchError::Impure { .. }), "{err:?}");
}

#[test]
fn assignment_inside_block_rejected() {
    let err = validate("{ x = 5; x }").unwrap_err();
    assert!(matches!(err, WatchError::Impure { .. }), "{err:?}");
}
