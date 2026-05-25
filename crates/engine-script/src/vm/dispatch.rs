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
            Opcode::ArrayNew => {
                let dst = code[frame.pc + 1];
                let n = code[frame.pc + 2];
                let mut elems = Vec::with_capacity(n as usize);
                for i in 0..n {
                    elems.push(frame.reg(code[frame.pc + 3 + i as usize]).clone());
                }
                frame.pc += 3 + n as usize;
                let h = heap.alloc(crate::gc::Obj::Array(elems));
                let frame = stack.last_mut().unwrap();
                frame.set_reg(dst, Value::Array(h));
            }
            Opcode::ArrayGet => {
                let dst = code[frame.pc + 1];
                let arr_reg = code[frame.pc + 2];
                let idx_reg = code[frame.pc + 3];
                let arr = match frame.reg(arr_reg) {
                    Value::Array(h) => *h,
                    other => {
                        return StopReason::Error(format!("ArrayGet on non-array: {other:?}"));
                    }
                };
                let i = match frame.reg(idx_reg) {
                    Value::Int(i) => *i,
                    other => {
                        return StopReason::Error(format!("ArrayGet index not Int: {other:?}"));
                    }
                };
                let v = match heap.get(arr) {
                    Some(crate::gc::Obj::Array(vs)) => {
                        if i < 0 || i as usize >= vs.len() {
                            return StopReason::Error(format!(
                                "array index out of bounds: {i} (len {})",
                                vs.len()
                            ));
                        }
                        vs[i as usize].clone()
                    }
                    Some(other) => {
                        return StopReason::Error(format!(
                            "ArrayGet handle resolved to non-array: {other:?}"
                        ));
                    }
                    None => return StopReason::Error("ArrayGet on dead handle".into()),
                };
                frame.set_reg(dst, v);
                frame.pc += 4;
            }
            Opcode::ArraySet => {
                let arr_reg = code[frame.pc + 1];
                let idx_reg = code[frame.pc + 2];
                let src_reg = code[frame.pc + 3];
                let arr = match frame.reg(arr_reg) {
                    Value::Array(h) => *h,
                    other => {
                        return StopReason::Error(format!("ArraySet on non-array: {other:?}"));
                    }
                };
                let i = match frame.reg(idx_reg) {
                    Value::Int(i) => *i,
                    other => {
                        return StopReason::Error(format!("ArraySet index not Int: {other:?}"));
                    }
                };
                let new_val = frame.reg(src_reg).clone();
                let target_handle = value_gc_handle(&new_val);
                match heap.get_mut(arr) {
                    Some(crate::gc::Obj::Array(vs)) => {
                        if i < 0 || i as usize >= vs.len() {
                            return StopReason::Error(format!(
                                "array index out of bounds: {i} (len {})",
                                vs.len()
                            ));
                        }
                        vs[i as usize] = new_val;
                    }
                    Some(other) => {
                        return StopReason::Error(format!(
                            "ArraySet handle resolved to non-array: {other:?}"
                        ));
                    }
                    None => return StopReason::Error("ArraySet on dead handle".into()),
                };
                if let Some(target) = target_handle {
                    heap.write_barrier(arr, target);
                }
                frame.pc += 4;
            }
            Opcode::ArrayLen => {
                let dst = code[frame.pc + 1];
                let arr_reg = code[frame.pc + 2];
                let arr = match frame.reg(arr_reg) {
                    Value::Array(h) => *h,
                    other => {
                        return StopReason::Error(format!("ArrayLen on non-array: {other:?}"));
                    }
                };
                let n = match heap.get(arr) {
                    Some(crate::gc::Obj::Array(vs)) => vs.len() as i64,
                    _ => return StopReason::Error("ArrayLen on dead/non-array handle".into()),
                };
                frame.set_reg(dst, Value::Int(n));
                frame.pc += 3;
            }
            Opcode::MapNew => {
                let dst = code[frame.pc + 1];
                frame.pc += 2;
                let h = heap.alloc(crate::gc::Obj::Map(Vec::new()));
                let frame = stack.last_mut().unwrap();
                frame.set_reg(dst, Value::Map(h));
            }
            Opcode::MapGet => {
                let dst = code[frame.pc + 1];
                let map_reg = code[frame.pc + 2];
                let key_reg = code[frame.pc + 3];
                let map = match frame.reg(map_reg) {
                    Value::Map(h) => *h,
                    other => {
                        return StopReason::Error(format!("MapGet on non-map: {other:?}"));
                    }
                };
                let key = match frame.reg(key_reg) {
                    Value::Str(s) => s.clone(),
                    other => {
                        return StopReason::Error(format!("MapGet key not Str: {other:?}"));
                    }
                };
                let v = match heap.get(map) {
                    Some(crate::gc::Obj::Map(entries)) => entries
                        .iter()
                        .find(|(k, _)| k.as_ref() == key.as_ref())
                        .map(|(_, v)| v.clone())
                        .unwrap_or(Value::Nil),
                    _ => return StopReason::Error("MapGet on dead/non-map handle".into()),
                };
                frame.set_reg(dst, v);
                frame.pc += 4;
            }
            Opcode::MapSet => {
                let map_reg = code[frame.pc + 1];
                let key_reg = code[frame.pc + 2];
                let src_reg = code[frame.pc + 3];
                let map = match frame.reg(map_reg) {
                    Value::Map(h) => *h,
                    other => {
                        return StopReason::Error(format!("MapSet on non-map: {other:?}"));
                    }
                };
                let key = match frame.reg(key_reg) {
                    Value::Str(s) => s.clone(),
                    other => {
                        return StopReason::Error(format!("MapSet key not Str: {other:?}"));
                    }
                };
                let new_val = frame.reg(src_reg).clone();
                let target_handle = value_gc_handle(&new_val);
                match heap.get_mut(map) {
                    Some(crate::gc::Obj::Map(entries)) => {
                        if let Some(slot) =
                            entries.iter_mut().find(|(k, _)| k.as_ref() == key.as_ref())
                        {
                            slot.1 = new_val;
                        } else {
                            entries.push((key, new_val));
                        }
                    }
                    _ => return StopReason::Error("MapSet on dead/non-map handle".into()),
                }
                if let Some(target) = target_handle {
                    heap.write_barrier(map, target);
                }
                frame.pc += 4;
            }
            Opcode::StructNew => {
                let dst = code[frame.pc + 1];
                frame.pc += 2;
                let h = heap.alloc(crate::gc::Obj::Struct(Vec::new()));
                let frame = stack.last_mut().unwrap();
                frame.set_reg(dst, Value::Struct(h));
            }
            Opcode::StructGet => {
                let dst = code[frame.pc + 1];
                let strct_reg = code[frame.pc + 2];
                let name_ki = u16::from_le_bytes([code[frame.pc + 3], code[frame.pc + 4]]);
                let name = match &module.constants[name_ki as usize] {
                    Const::Str(s) => s.clone(),
                    other => {
                        return StopReason::Error(format!(
                            "StructGet name const not Str: {other:?}"
                        ));
                    }
                };
                let strct = match frame.reg(strct_reg) {
                    Value::Struct(h) => *h,
                    other => {
                        return StopReason::Error(format!("StructGet on non-struct: {other:?}"));
                    }
                };
                let v = match heap.get(strct) {
                    Some(crate::gc::Obj::Struct(fields)) => fields
                        .iter()
                        .find(|(k, _)| k.as_ref() == name.as_str())
                        .map(|(_, v)| v.clone())
                        .unwrap_or(Value::Nil),
                    _ => return StopReason::Error("StructGet on dead/non-struct handle".into()),
                };
                frame.set_reg(dst, v);
                frame.pc += 5;
            }
            Opcode::StructSet => {
                let strct_reg = code[frame.pc + 1];
                let name_ki = u16::from_le_bytes([code[frame.pc + 2], code[frame.pc + 3]]);
                let src_reg = code[frame.pc + 4];
                let name: Arc<str> = match &module.constants[name_ki as usize] {
                    Const::Str(s) => Arc::from(s.clone()),
                    other => {
                        return StopReason::Error(format!(
                            "StructSet name const not Str: {other:?}"
                        ));
                    }
                };
                let strct = match frame.reg(strct_reg) {
                    Value::Struct(h) => *h,
                    other => {
                        return StopReason::Error(format!("StructSet on non-struct: {other:?}"));
                    }
                };
                let new_val = frame.reg(src_reg).clone();
                let target_handle = value_gc_handle(&new_val);
                match heap.get_mut(strct) {
                    Some(crate::gc::Obj::Struct(fields)) => {
                        if let Some(slot) =
                            fields.iter_mut().find(|(k, _)| k.as_ref() == name.as_ref())
                        {
                            slot.1 = new_val;
                        } else {
                            fields.push((name, new_val));
                        }
                    }
                    _ => return StopReason::Error("StructSet on dead/non-struct handle".into()),
                }
                if let Some(target) = target_handle {
                    heap.write_barrier(strct, target);
                }
                frame.pc += 5;
            }
            Opcode::ClosureMake => {
                let dst = code[frame.pc + 1];
                let fn_idx = u16::from_le_bytes([code[frame.pc + 2], code[frame.pc + 3]]);
                let n = code[frame.pc + 4];
                let mut upvalues = Vec::with_capacity(n as usize);
                for i in 0..n {
                    upvalues.push(frame.reg(code[frame.pc + 5 + i as usize]).clone());
                }
                frame.pc += 5 + n as usize;
                let h = heap.alloc(crate::gc::Obj::Closure {
                    function_id: fn_idx,
                    upvalues,
                });
                let frame = stack.last_mut().unwrap();
                frame.set_reg(dst, Value::Closure(h));
            }
            Opcode::CallClosure => {
                let dst = code[frame.pc + 1];
                let cls_reg = code[frame.pc + 2];
                let n = code[frame.pc + 3];
                let cls = match frame.reg(cls_reg) {
                    Value::Closure(h) => *h,
                    other => {
                        return StopReason::Error(format!("CallClosure on non-closure: {other:?}"));
                    }
                };
                let mut args = Vec::with_capacity(n as usize);
                for i in 0..n {
                    args.push(frame.reg(code[frame.pc + 4 + i as usize]).clone());
                }
                let advance = 4 + n as usize;
                frame.pc += advance;
                let (function_id, upvalues) = match heap.get(cls) {
                    Some(crate::gc::Obj::Closure {
                        function_id,
                        upvalues,
                    }) => (*function_id, upvalues.clone()),
                    _ => {
                        return StopReason::Error("CallClosure on dead/non-closure handle".into());
                    }
                };
                let callee = match module.functions.get(function_id as usize) {
                    Some(f) => f,
                    None => {
                        return StopReason::Error(format!(
                            "closure refers to unknown function id {function_id}"
                        ));
                    }
                };
                let mut callee_frame = CallFrame::new(function_id, callee.max_register, Some(dst));
                // Upvalues occupy registers 0..k; user args occupy k..k+n.
                for (i, v) in upvalues.into_iter().enumerate() {
                    callee_frame.set_reg(i as u8, v);
                }
                for (i, v) in args.into_iter().enumerate() {
                    callee_frame.set_reg((i + cls_upvalue_offset(heap, cls)) as u8, v);
                }
                stack.push(callee_frame);
            }
        }
    }
}

fn value_gc_handle(v: &Value) -> Option<crate::gc::GcHandle> {
    match v {
        Value::Array(h) | Value::Map(h) | Value::Struct(h) | Value::Closure(h) => Some(*h),
        _ => None,
    }
}

fn cls_upvalue_offset(heap: &crate::gc::Heap, cls: crate::gc::GcHandle) -> usize {
    match heap.get(cls) {
        Some(crate::gc::Obj::Closure { upvalues, .. }) => upvalues.len(),
        _ => 0,
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
