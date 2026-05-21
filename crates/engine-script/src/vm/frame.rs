//! Per-call frame state for the dispatch loop.

use crate::vm::Value;

/// One call frame.
#[derive(Debug)]
pub struct CallFrame {
    /// Function id in the module's function table.
    pub function_id: u16,
    /// Program counter — byte offset into the function's code.
    pub pc: usize,
    /// Register window. Sized to the function's `max_register` count.
    pub registers: Vec<Value>,
    /// Where to write the call's return value when `ReturnVal` fires.
    /// The outermost frame has `None` (the value escapes to the caller
    /// of `Vm::call`).
    pub return_dst: Option<u8>,
}

impl CallFrame {
    /// Constructs a frame ready to execute the function's first opcode.
    pub fn new(function_id: u16, register_count: u8, return_dst: Option<u8>) -> Self {
        Self {
            function_id,
            pc: 0,
            registers: vec![Value::Nil; register_count as usize],
            return_dst,
        }
    }

    /// Reads register `r`. Verifier already guarantees `r < registers.len()`.
    #[inline(always)]
    pub fn reg(&self, r: u8) -> &Value {
        &self.registers[r as usize]
    }

    /// Writes register `r`.
    #[inline(always)]
    pub fn set_reg(&mut self, r: u8, v: Value) {
        self.registers[r as usize] = v;
    }
}
