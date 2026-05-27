//! Bytecode verifier (ADR-035).
//!
//! Walks every function's code stream and enforces:
//!
//! 1. The first byte of every step is a known opcode.
//! 2. **No code byte is `0xFF`.** The TRAP opcode is reserved for the
//!    debugger; user code that contains `0xFF` cannot execute. This is
//!    the runtime backstop for the architectural argument in
//!    `bytecode.rs` (4-layer enforcement, ADR-035).
//! 3. Every register operand satisfies `r < max_register`.
//! 4. Every const-pool index is in range.
//! 5. Every jump target lands on an opcode boundary, in range.
//! 6. Every function-id operand is in range.
//! 7. The function ends with a return (`ReturnNil`, `ReturnVal`) or a
//!    valid jump back into the function (loops are allowed; falling
//!    off the end is not).
//!
//! Stack-balance and type-tag consistency the design calls for ride on
//! the type-checker (PR 1) for now — PR 2 surfaces the verifier as the
//! gate to execution, leaving room for the stricter checks to land
//! without changing the API.

use crate::bytecode::{Module, Opcode};

/// What went wrong during verification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VerifyError {
    /// Unknown opcode byte encountered at `pc`.
    UnknownOpcode {
        /// Function whose code violates the invariant.
        function: String,
        /// Byte offset inside that function's code.
        pc: usize,
        /// The offending byte value.
        byte: u8,
    },
    /// TRAP opcode found in user code. The verifier rejects this even
    /// though `Opcode::from_u8(0xFF)` succeeds — only the debugger may
    /// patch a TRAP in, and that happens *after* verification.
    TrapInCode {
        /// Function whose code violates the invariant.
        function: String,
        /// Byte offset of the TRAP byte.
        pc: usize,
    },
    /// Register operand out of bounds.
    OutOfBoundsRegister {
        /// Function whose code violates the invariant.
        function: String,
        /// Byte offset of the opcode whose register operand failed.
        pc: usize,
        /// The offending register index.
        reg: u8,
        /// The function's declared `max_register`.
        max: u8,
    },
    /// Const-pool operand out of bounds.
    OutOfBoundsConst {
        /// Function whose code violates the invariant.
        function: String,
        /// Byte offset of the opcode whose const-pool operand failed.
        pc: usize,
        /// The offending const-pool index.
        idx: u16,
        /// The module's const-pool size.
        max: u16,
    },
    /// Function-id operand out of bounds.
    OutOfBoundsFunction {
        /// Function whose code violates the invariant.
        function: String,
        /// Byte offset of the opcode whose function-id operand failed.
        pc: usize,
        /// The offending function id.
        id: u16,
        /// The module's function-table size.
        max: u16,
    },
    /// Jump target outside the function's code or off an opcode boundary.
    BadJumpTarget {
        /// Function whose code violates the invariant.
        function: String,
        /// Byte offset of the jump opcode.
        pc: usize,
        /// Computed absolute target byte offset.
        target: isize,
    },
    /// Truncated instruction — operand bytes ran past EOF.
    Truncated {
        /// Function whose code violates the invariant.
        function: String,
        /// Byte offset at which the truncation was detected.
        pc: usize,
    },
    /// Function falls off the end without an explicit return.
    MissingReturn {
        /// Function whose code violates the invariant.
        function: String,
    },
}

impl std::fmt::Display for VerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownOpcode { function, pc, byte } => {
                write!(f, "{function}@{pc}: unknown opcode 0x{byte:02x}")
            }
            Self::TrapInCode { function, pc } => {
                write!(f, "{function}@{pc}: TRAP reserved for debugger (ADR-035)")
            }
            Self::OutOfBoundsRegister {
                function,
                pc,
                reg,
                max,
            } => {
                write!(f, "{function}@{pc}: register r{reg} >= max r{max}")
            }
            Self::OutOfBoundsConst {
                function,
                pc,
                idx,
                max,
            } => {
                write!(f, "{function}@{pc}: const idx {idx} >= pool size {max}")
            }
            Self::OutOfBoundsFunction {
                function,
                pc,
                id,
                max,
            } => {
                write!(f, "{function}@{pc}: function id {id} >= table size {max}")
            }
            Self::BadJumpTarget {
                function,
                pc,
                target,
            } => {
                write!(f, "{function}@{pc}: jump target {target} out of range")
            }
            Self::Truncated { function, pc } => {
                write!(f, "{function}@{pc}: truncated instruction")
            }
            Self::MissingReturn { function } => {
                write!(f, "{function}: missing return")
            }
        }
    }
}

impl std::error::Error for VerifyError {}

/// Verifies every function in `module`. Returns `Ok(())` on success or
/// the first failure encountered.
pub fn verify(module: &Module) -> Result<(), VerifyError> {
    for f in &module.functions {
        verify_function(f, module)?;
    }
    Ok(())
}

fn verify_function(
    f: &crate::bytecode::FunctionBytecode,
    module: &Module,
) -> Result<(), VerifyError> {
    let code = &f.code;
    let max_reg = f.max_register;
    let const_max = module.constants.len() as u16;
    let fn_max = module.functions.len() as u16;
    // First pass: catalogue opcode boundaries.
    let mut boundaries = vec![false; code.len() + 1];
    boundaries[0] = true;
    let mut pc = 0;
    while pc < code.len() {
        let byte = code[pc];
        if byte == 0xFF {
            return Err(VerifyError::TrapInCode {
                function: f.name.clone(),
                pc,
            });
        }
        let Some(op) = Opcode::from_u8(byte) else {
            return Err(VerifyError::UnknownOpcode {
                function: f.name.clone(),
                pc,
                byte,
            });
        };
        let len = op.instr_len(code, pc);
        if pc + len > code.len() {
            return Err(VerifyError::Truncated {
                function: f.name.clone(),
                pc,
            });
        }
        // Operand bounds.
        match op {
            Opcode::Trap | Opcode::Nop | Opcode::ReturnNil => {}
            Opcode::ConstNil | Opcode::ConstTrue | Opcode::ConstFalse | Opcode::ReturnVal => {
                let r = code[pc + 1];
                check_reg(&f.name, pc, r, max_reg)?;
            }
            Opcode::Move | Opcode::Not | Opcode::Neg => {
                check_reg(&f.name, pc, code[pc + 1], max_reg)?;
                check_reg(&f.name, pc, code[pc + 2], max_reg)?;
            }
            Opcode::ConstInt | Opcode::ConstFloat | Opcode::ConstStr => {
                check_reg(&f.name, pc, code[pc + 1], max_reg)?;
                let idx = u16::from_le_bytes([code[pc + 2], code[pc + 3]]);
                if idx >= const_max {
                    return Err(VerifyError::OutOfBoundsConst {
                        function: f.name.clone(),
                        pc,
                        idx,
                        max: const_max,
                    });
                }
            }
            Opcode::Add
            | Opcode::Sub
            | Opcode::Mul
            | Opcode::Div
            | Opcode::Mod
            | Opcode::Eq
            | Opcode::Ne
            | Opcode::Lt
            | Opcode::Le
            | Opcode::Gt
            | Opcode::Ge
            | Opcode::And
            | Opcode::Or => {
                check_reg(&f.name, pc, code[pc + 1], max_reg)?;
                check_reg(&f.name, pc, code[pc + 2], max_reg)?;
                check_reg(&f.name, pc, code[pc + 3], max_reg)?;
            }
            Opcode::Jmp => {
                let off = i16::from_le_bytes([code[pc + 1], code[pc + 2]]) as isize;
                let target = pc as isize + 3 + off;
                if target < 0 || target as usize > code.len() {
                    return Err(VerifyError::BadJumpTarget {
                        function: f.name.clone(),
                        pc,
                        target,
                    });
                }
            }
            Opcode::JmpIfFalse | Opcode::JmpIfTrue => {
                check_reg(&f.name, pc, code[pc + 1], max_reg)?;
                let off = i16::from_le_bytes([code[pc + 2], code[pc + 3]]) as isize;
                let target = pc as isize + 4 + off;
                if target < 0 || target as usize > code.len() {
                    return Err(VerifyError::BadJumpTarget {
                        function: f.name.clone(),
                        pc,
                        target,
                    });
                }
            }
            Opcode::Call => {
                check_reg(&f.name, pc, code[pc + 1], max_reg)?;
                let id = u16::from_le_bytes([code[pc + 2], code[pc + 3]]);
                if id >= fn_max {
                    return Err(VerifyError::OutOfBoundsFunction {
                        function: f.name.clone(),
                        pc,
                        id,
                        max: fn_max,
                    });
                }
                let n = code[pc + 4];
                for i in 0..n {
                    check_reg(&f.name, pc, code[pc + 5 + i as usize], max_reg)?;
                }
            }
            Opcode::FfiCall => {
                check_reg(&f.name, pc, code[pc + 1], max_reg)?;
                // FFI index bound check is deferred to `Vm::call` since
                // the table is registered at runtime, not at module
                // load.
                let n = code[pc + 4];
                for i in 0..n {
                    check_reg(&f.name, pc, code[pc + 5 + i as usize], max_reg)?;
                }
            }
            // Aggregate opcodes (ADR-060). Register-only checks; type
            // and key checks are deferred to runtime (the verifier
            // does not carry a type lattice for aggregates today).
            Opcode::ArrayNew => {
                check_reg(&f.name, pc, code[pc + 1], max_reg)?;
                let n = code[pc + 2];
                for i in 0..n {
                    check_reg(&f.name, pc, code[pc + 3 + i as usize], max_reg)?;
                }
            }
            Opcode::ArrayGet | Opcode::ArraySet | Opcode::MapGet | Opcode::MapSet => {
                check_reg(&f.name, pc, code[pc + 1], max_reg)?;
                check_reg(&f.name, pc, code[pc + 2], max_reg)?;
                check_reg(&f.name, pc, code[pc + 3], max_reg)?;
            }
            Opcode::ArrayLen | Opcode::MapNew | Opcode::StructNew => {
                check_reg(&f.name, pc, code[pc + 1], max_reg)?;
                if op == Opcode::ArrayLen {
                    check_reg(&f.name, pc, code[pc + 2], max_reg)?;
                }
            }
            Opcode::StructGet => {
                // Layout: `dst:u8 strct:u8 name_ki:u16le` (dispatcher
                // reads pc+1=dst, pc+2=strct, pc+3..pc+5=name_ki).
                check_reg(&f.name, pc, code[pc + 1], max_reg)?;
                check_reg(&f.name, pc, code[pc + 2], max_reg)?;
                let name_ki = u16::from_le_bytes([code[pc + 3], code[pc + 4]]);
                if name_ki >= const_max {
                    return Err(VerifyError::OutOfBoundsConst {
                        function: f.name.clone(),
                        pc,
                        idx: name_ki,
                        max: const_max,
                    });
                }
            }
            Opcode::StructSet => {
                // Layout: `strct:u8 name_ki:u16le src:u8` (dispatcher
                // reads pc+1=strct, pc+2..pc+4=name_ki, pc+4=src).
                check_reg(&f.name, pc, code[pc + 1], max_reg)?;
                let name_ki = u16::from_le_bytes([code[pc + 2], code[pc + 3]]);
                if name_ki >= const_max {
                    return Err(VerifyError::OutOfBoundsConst {
                        function: f.name.clone(),
                        pc,
                        idx: name_ki,
                        max: const_max,
                    });
                }
                check_reg(&f.name, pc, code[pc + 4], max_reg)?;
            }
            Opcode::ClosureMake => {
                check_reg(&f.name, pc, code[pc + 1], max_reg)?;
                let id = u16::from_le_bytes([code[pc + 2], code[pc + 3]]);
                if id >= fn_max {
                    return Err(VerifyError::OutOfBoundsFunction {
                        function: f.name.clone(),
                        pc,
                        id,
                        max: fn_max,
                    });
                }
                let n = code[pc + 4];
                for i in 0..n {
                    check_reg(&f.name, pc, code[pc + 5 + i as usize], max_reg)?;
                }
            }
            Opcode::CallClosure => {
                check_reg(&f.name, pc, code[pc + 1], max_reg)?;
                check_reg(&f.name, pc, code[pc + 2], max_reg)?;
                let n = code[pc + 3];
                for i in 0..n {
                    check_reg(&f.name, pc, code[pc + 4 + i as usize], max_reg)?;
                }
            }
        }
        pc += len;
        if pc <= code.len() {
            boundaries[pc] = true;
        }
    }
    // Second pass: re-check jump targets against boundaries.
    let mut pc = 0;
    while pc < code.len() {
        let op = Opcode::from_u8(code[pc]).expect("first pass already checked");
        match op {
            Opcode::Jmp => {
                let off = i16::from_le_bytes([code[pc + 1], code[pc + 2]]) as isize;
                let target = pc as isize + 3 + off;
                if !boundaries.get(target as usize).copied().unwrap_or(false) {
                    return Err(VerifyError::BadJumpTarget {
                        function: f.name.clone(),
                        pc,
                        target,
                    });
                }
            }
            Opcode::JmpIfFalse | Opcode::JmpIfTrue => {
                let off = i16::from_le_bytes([code[pc + 2], code[pc + 3]]) as isize;
                let target = pc as isize + 4 + off;
                if !boundaries.get(target as usize).copied().unwrap_or(false) {
                    return Err(VerifyError::BadJumpTarget {
                        function: f.name.clone(),
                        pc,
                        target,
                    });
                }
            }
            _ => {}
        }
        pc += op.instr_len(code, pc);
    }
    // Last opcode must be a return.
    let mut last_pc = 0;
    let mut pc = 0;
    while pc < code.len() {
        last_pc = pc;
        let op = Opcode::from_u8(code[pc]).unwrap();
        pc += op.instr_len(code, pc);
    }
    if code.is_empty() {
        return Err(VerifyError::MissingReturn {
            function: f.name.clone(),
        });
    }
    let last_op = Opcode::from_u8(code[last_pc]).unwrap();
    if !matches!(last_op, Opcode::ReturnNil | Opcode::ReturnVal) {
        return Err(VerifyError::MissingReturn {
            function: f.name.clone(),
        });
    }
    Ok(())
}

fn check_reg(function: &str, pc: usize, reg: u8, max: u8) -> Result<(), VerifyError> {
    if reg >= max {
        return Err(VerifyError::OutOfBoundsRegister {
            function: function.to_string(),
            pc,
            reg,
            max,
        });
    }
    Ok(())
}
