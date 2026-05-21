//! Dispatch loop for the sli register VM.
//!
//! Threaded dispatch — the match arms are `#[inline(always)]` so a
//! release build folds the inner loop into a contiguous decision tree
//! (the GCC computed-goto trick that LLVM mimics for a tightly-packed
//! match on a `u8`). Portable Rust; no inline assembly required.

use crate::bytecode::{Const, Module, Opcode};
use crate::ffi::CallTable;
use crate::gc::Heap;
use crate::vm::{CallFrame, Value};
use std::sync::Arc;

/// Why a fiber stopped executing.
#[derive(Clone, Debug, PartialEq)]
pub enum StopReason {
    /// The outermost frame returned. Carries the return value.
    Returned(Value),
    /// The fiber hit a `Trap` opcode (debugger patch).
    Trapped {
        /// Function id where the trap fired.
        function_id: u16,
        /// Byte offset of the trap inside that function.
        pc: usize,
    },
    /// A runtime error the verifier could not catch: division by zero,
    /// integer overflow on `i64::MIN.wrapping_neg()`-like edge cases,
    /// or a deliberate `panic!` from FFI.
    Error(String),
}

/// One frame of dispatch work — runs until the stack is empty or a
/// non-returning [`StopReason`] is produced.
pub fn run(
    module: &Module,
    stack: &mut Vec<CallFrame>,
    heap: &mut Heap,
    ffi: &CallTable,
) -> StopReason {
    loop {
        let top_idx = stack.len() - 1;
        let frame = &mut stack[top_idx];
        let function = &module.functions[frame.function_id as usize];
        let code = &function.code;
        if frame.pc >= code.len() {
            // Implicit `return nil` at end of code.
            let v = Value::Nil;
            if let Some(_dst) = frame.return_dst {
                let dst = frame.return_dst.unwrap();
                stack.pop();
                if let Some(caller) = stack.last_mut() {
                    caller.set_reg(dst, v);
                    continue;
                }
                return StopReason::Returned(Value::Nil);
            }
            stack.pop();
            return StopReason::Returned(Value::Nil);
        }
        let opcode_byte = code[frame.pc];
        let Some(op) = Opcode::from_u8(opcode_byte) else {
            return StopReason::Error(format!("unknown opcode 0x{opcode_byte:02x}"));
        };
        match op {
            Opcode::Trap => {
                let pc = frame.pc;
                let function_id = frame.function_id;
                return StopReason::Trapped { function_id, pc };
            }
            Opcode::Nop => frame.pc += 1,
            Opcode::ConstNil => {
                let dst = code[frame.pc + 1];
                frame.set_reg(dst, Value::Nil);
                frame.pc += 2;
            }
            Opcode::ConstTrue => {
                let dst = code[frame.pc + 1];
                frame.set_reg(dst, Value::Bool(true));
                frame.pc += 2;
            }
            Opcode::ConstFalse => {
                let dst = code[frame.pc + 1];
                frame.set_reg(dst, Value::Bool(false));
                frame.pc += 2;
            }
            Opcode::ConstInt => {
                let dst = code[frame.pc + 1];
                let idx = u16::from_le_bytes([code[frame.pc + 2], code[frame.pc + 3]]);
                let v = match &module.constants[idx as usize] {
                    Const::Int(v) => Value::Int(*v),
                    other => {
                        return StopReason::Error(format!("const pool mismatch: {other:?}"));
                    }
                };
                frame.set_reg(dst, v);
                frame.pc += 4;
            }
            Opcode::ConstFloat => {
                let dst = code[frame.pc + 1];
                let idx = u16::from_le_bytes([code[frame.pc + 2], code[frame.pc + 3]]);
                let v = match &module.constants[idx as usize] {
                    Const::Float(b) => Value::Float(f64::from_bits(*b)),
                    other => {
                        return StopReason::Error(format!("const pool mismatch: {other:?}"));
                    }
                };
                frame.set_reg(dst, v);
                frame.pc += 4;
            }
            Opcode::ConstStr => {
                let dst = code[frame.pc + 1];
                let idx = u16::from_le_bytes([code[frame.pc + 2], code[frame.pc + 3]]);
                let s = match &module.constants[idx as usize] {
                    Const::Str(s) => s.clone(),
                    other => {
                        return StopReason::Error(format!("const pool mismatch: {other:?}"));
                    }
                };
                frame.set_reg(dst, Value::Str(Arc::from(s)));
                frame.pc += 4;
            }
            Opcode::Move => {
                let dst = code[frame.pc + 1];
                let src = code[frame.pc + 2];
                let v = frame.reg(src).clone();
                frame.set_reg(dst, v);
                frame.pc += 3;
            }
            Opcode::Add | Opcode::Sub | Opcode::Mul | Opcode::Div | Opcode::Mod => {
                let dst = code[frame.pc + 1];
                let a = frame.reg(code[frame.pc + 2]).clone();
                let b = frame.reg(code[frame.pc + 3]).clone();
                let v = match (&a, &b) {
                    (Value::Int(x), Value::Int(y)) => match op {
                        Opcode::Add => Value::Int(x.wrapping_add(*y)),
                        Opcode::Sub => Value::Int(x.wrapping_sub(*y)),
                        Opcode::Mul => Value::Int(x.wrapping_mul(*y)),
                        Opcode::Div => match x.checked_div(*y) {
                            Some(v) => Value::Int(v),
                            None => return StopReason::Error("integer division by zero".into()),
                        },
                        Opcode::Mod => match x.checked_rem(*y) {
                            Some(v) => Value::Int(v),
                            None => return StopReason::Error("integer modulo by zero".into()),
                        },
                        _ => unreachable!(),
                    },
                    (Value::Float(x), Value::Float(y)) => match op {
                        Opcode::Add => Value::Float(x + y),
                        Opcode::Sub => Value::Float(x - y),
                        Opcode::Mul => Value::Float(x * y),
                        Opcode::Div => Value::Float(x / y),
                        Opcode::Mod => Value::Float(x % y),
                        _ => unreachable!(),
                    },
                    _ => {
                        return StopReason::Error(format!(
                            "arithmetic on incompatible values: {a:?} {op:?} {b:?}"
                        ));
                    }
                };
                frame.set_reg(dst, v);
                frame.pc += 4;
            }
            Opcode::Neg => {
                let dst = code[frame.pc + 1];
                let src = code[frame.pc + 2];
                let v = match frame.reg(src) {
                    Value::Int(x) => Value::Int(x.wrapping_neg()),
                    Value::Float(x) => Value::Float(-x),
                    other => {
                        return StopReason::Error(format!("neg on non-numeric: {other:?}"));
                    }
                };
                frame.set_reg(dst, v);
                frame.pc += 3;
            }
            Opcode::Eq | Opcode::Ne | Opcode::Lt | Opcode::Le | Opcode::Gt | Opcode::Ge => {
                let dst = code[frame.pc + 1];
                let a = frame.reg(code[frame.pc + 2]).clone();
                let b = frame.reg(code[frame.pc + 3]).clone();
                let b_val = match op {
                    Opcode::Eq => a == b,
                    Opcode::Ne => a != b,
                    _ => compare_ordered(op, &a, &b)
                        .ok_or_else(|| format!("cannot order {a:?} and {b:?}"))
                        .unwrap_or(false),
                };
                frame.set_reg(dst, Value::Bool(b_val));
                frame.pc += 4;
            }
            Opcode::Not => {
                let dst = code[frame.pc + 1];
                let src = code[frame.pc + 2];
                let v = match frame.reg(src) {
                    Value::Bool(b) => Value::Bool(!b),
                    other => {
                        return StopReason::Error(format!("not on non-bool: {other:?}"));
                    }
                };
                frame.set_reg(dst, v);
                frame.pc += 3;
            }
            Opcode::And | Opcode::Or => {
                let dst = code[frame.pc + 1];
                let a = frame.reg(code[frame.pc + 2]).clone();
                let b = frame.reg(code[frame.pc + 3]).clone();
                let v = match (&a, &b) {
                    (Value::Bool(x), Value::Bool(y)) => match op {
                        Opcode::And => Value::Bool(*x && *y),
                        Opcode::Or => Value::Bool(*x || *y),
                        _ => unreachable!(),
                    },
                    _ => {
                        return StopReason::Error(format!("logical on non-bool: {a:?} / {b:?}"));
                    }
                };
                frame.set_reg(dst, v);
                frame.pc += 4;
            }
            Opcode::Jmp => {
                let off = i16::from_le_bytes([code[frame.pc + 1], code[frame.pc + 2]]) as isize;
                let next = (frame.pc as isize + 3 + off) as usize;
                frame.pc = next;
            }
            Opcode::JmpIfFalse => {
                let cond = frame.reg(code[frame.pc + 1]).clone();
                let off = i16::from_le_bytes([code[frame.pc + 2], code[frame.pc + 3]]) as isize;
                let taken = matches!(cond, Value::Bool(false));
                frame.pc = if taken {
                    (frame.pc as isize + 4 + off) as usize
                } else {
                    frame.pc + 4
                };
            }
            Opcode::JmpIfTrue => {
                let cond = frame.reg(code[frame.pc + 1]).clone();
                let off = i16::from_le_bytes([code[frame.pc + 2], code[frame.pc + 3]]) as isize;
                let taken = matches!(cond, Value::Bool(true));
                frame.pc = if taken {
                    (frame.pc as isize + 4 + off) as usize
                } else {
                    frame.pc + 4
                };
            }
            Opcode::Call => {
                let dst = code[frame.pc + 1];
                let fn_idx = u16::from_le_bytes([code[frame.pc + 2], code[frame.pc + 3]]);
                let n = code[frame.pc + 4];
                let mut args = Vec::with_capacity(n as usize);
                for i in 0..n {
                    args.push(frame.reg(code[frame.pc + 5 + i as usize]).clone());
                }
                let advance = 5 + n as usize;
                frame.pc += advance;
                let callee = match module.functions.get(fn_idx as usize) {
                    Some(f) => f,
                    None => return StopReason::Error(format!("unknown function id {fn_idx}")),
                };
                let mut callee_frame = CallFrame::new(fn_idx, callee.max_register, Some(dst));
                for (i, v) in args.into_iter().enumerate() {
                    callee_frame.set_reg(i as u8, v);
                }
                stack.push(callee_frame);
            }
            Opcode::FfiCall => {
                let dst = code[frame.pc + 1];
                let ffi_idx = u16::from_le_bytes([code[frame.pc + 2], code[frame.pc + 3]]);
                let n = code[frame.pc + 4];
                let mut args = Vec::with_capacity(n as usize);
                for i in 0..n {
                    args.push(frame.reg(code[frame.pc + 5 + i as usize]).clone());
                }
                frame.pc += 5 + n as usize;
                let v = match ffi.call(ffi_idx, &args, heap) {
                    Ok(v) => v,
                    Err(e) => return StopReason::Error(format!("ffi error: {e}")),
                };
                let frame = stack.last_mut().unwrap();
                frame.set_reg(dst, v);
            }
            Opcode::ReturnNil => {
                let dst = frame.return_dst;
                stack.pop();
                if let Some(caller) = stack.last_mut() {
                    if let Some(dst) = dst {
                        caller.set_reg(dst, Value::Nil);
                    }
                    continue;
                }
                return StopReason::Returned(Value::Nil);
            }
            Opcode::ReturnVal => {
                let src = code[frame.pc + 1];
                let v = frame.reg(src).clone();
                let dst = frame.return_dst;
                stack.pop();
                if let Some(caller) = stack.last_mut() {
                    if let Some(dst) = dst {
                        caller.set_reg(dst, v);
                    }
                    continue;
                }
                return StopReason::Returned(v);
            }
        }
    }
}

fn compare_ordered(op: Opcode, a: &Value, b: &Value) -> Option<bool> {
    let (x, y) = match (a, b) {
        (Value::Int(x), Value::Int(y)) => (*x as f64, *y as f64),
        (Value::Float(x), Value::Float(y)) => (*x, *y),
        _ => return None,
    };
    Some(match op {
        Opcode::Lt => x < y,
        Opcode::Le => x <= y,
        Opcode::Gt => x > y,
        Opcode::Ge => x >= y,
        _ => unreachable!(),
    })
}
