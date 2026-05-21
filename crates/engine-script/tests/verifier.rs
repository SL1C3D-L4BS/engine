//! Bytecode verifier negative cases (PR 2, ADR-035).
//!
//! Hand-built `FunctionBytecode` fixtures exercise each rejection
//! reason. Stack-balance and type-tag consistency the design calls
//! for are the type-checker's job (PR 1); the verifier is the gate
//! to execution and protects the invariants the dispatch loop assumes.

use engine_script::Opcode;
use engine_script::bytecode::{Const, FunctionBytecode, Module};
use engine_script::verify::{VerifyError, verify};

fn one_fn(name: &str, code: Vec<u8>, max_register: u8) -> Module {
    let function = FunctionBytecode {
        name: name.to_string(),
        arity: 0,
        max_register,
        code,
        line_for_pc: Vec::new(),
    };
    let function_index = vec![(name.to_string(), 0u16)];
    Module {
        constants: Vec::new(),
        functions: vec![function],
        function_index,
    }
}

#[test]
fn returns_ok_on_trivial_return() {
    let module = one_fn("ret_nil", vec![Opcode::ReturnNil as u8], 1);
    verify(&module).expect("trivial return must verify");
}

#[test]
fn trap_in_code_is_rejected() {
    // ConstNil r0; TRAP; ReturnNil
    let code = vec![Opcode::ConstNil as u8, 0, 0xFF, Opcode::ReturnNil as u8];
    let module = one_fn("trap", code, 1);
    let err = verify(&module).unwrap_err();
    assert!(matches!(err, VerifyError::TrapInCode { .. }), "{err:?}");
}

#[test]
fn unknown_opcode_is_rejected() {
    let code = vec![0xAA, Opcode::ReturnNil as u8];
    let module = one_fn("bad_op", code, 1);
    let err = verify(&module).unwrap_err();
    assert!(matches!(err, VerifyError::UnknownOpcode { .. }), "{err:?}");
}

#[test]
fn oob_register_is_rejected() {
    // ConstNil r5 — but max_register is 2.
    let code = vec![Opcode::ConstNil as u8, 5, Opcode::ReturnNil as u8];
    let module = one_fn("oob_reg", code, 2);
    let err = verify(&module).unwrap_err();
    assert!(
        matches!(err, VerifyError::OutOfBoundsRegister { .. }),
        "{err:?}"
    );
}

#[test]
fn oob_const_is_rejected() {
    // ConstInt r0, const_idx 99 — empty const pool.
    let code = vec![Opcode::ConstInt as u8, 0, 99, 0, Opcode::ReturnNil as u8];
    let module = one_fn("oob_const", code, 1);
    let err = verify(&module).unwrap_err();
    assert!(
        matches!(err, VerifyError::OutOfBoundsConst { .. }),
        "{err:?}"
    );
}

#[test]
fn oob_function_is_rejected() {
    // Call r0, fn_idx 7, 0 args
    let code = vec![Opcode::Call as u8, 0, 7, 0, 0, Opcode::ReturnNil as u8];
    let module = one_fn("oob_fn", code, 1);
    let err = verify(&module).unwrap_err();
    assert!(
        matches!(err, VerifyError::OutOfBoundsFunction { .. }),
        "{err:?}"
    );
}

#[test]
fn missing_return_is_rejected() {
    let code = vec![Opcode::ConstNil as u8, 0];
    let module = one_fn("no_ret", code, 1);
    let err = verify(&module).unwrap_err();
    assert!(matches!(err, VerifyError::MissingReturn { .. }), "{err:?}");
}

#[test]
fn truncated_instruction_is_rejected() {
    // ConstInt expects 3 more bytes after opcode but only has 1.
    let code = vec![Opcode::ConstInt as u8, 0];
    let module = one_fn("trunc", code, 1);
    let err = verify(&module).unwrap_err();
    assert!(matches!(err, VerifyError::Truncated { .. }), "{err:?}");
}

#[test]
fn bad_jump_target_is_rejected() {
    // Jmp by 100 in a 4-byte function.
    let code = vec![Opcode::Jmp as u8, 100, 0, Opcode::ReturnNil as u8];
    let module = one_fn("bad_jump", code, 1);
    let err = verify(&module).unwrap_err();
    assert!(matches!(err, VerifyError::BadJumpTarget { .. }), "{err:?}");
}

#[test]
fn well_formed_const_lookup_passes() {
    let code = vec![Opcode::ConstInt as u8, 0, 0, 0, Opcode::ReturnVal as u8, 0];
    let module = Module {
        constants: vec![Const::Int(42)],
        functions: vec![FunctionBytecode {
            name: "ok".to_string(),
            arity: 0,
            max_register: 1,
            code,
            line_for_pc: Vec::new(),
        }],
        function_index: vec![("ok".to_string(), 0)],
    };
    verify(&module).expect("well-formed const fetch must verify");
}
