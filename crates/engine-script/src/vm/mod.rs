//! The sli register VM.
//!
//! - [`Value`] is the tagged runtime value.
//! - [`CallFrame`] is the per-call register window.
//! - [`Vm`] owns the module, GC heap, and FFI table; [`Vm::run`] is the
//!   single entry point for executing a named entry function.
//! - [`dispatch::run`] is the threaded dispatch loop.

pub mod dispatch;
mod frame;
mod value;

pub use dispatch::{StopReason, run};
pub use frame::CallFrame;
pub use value::{Value, summary};

use crate::bytecode::Module;
use crate::ffi::CallTable;
use crate::gc::Heap;

/// A running VM. Holds the loaded module, the GC heap, and the FFI table.
#[derive(Debug)]
pub struct Vm {
    /// Compiled module.
    pub module: Module,
    /// Owned heap — strings, structs, arrays, maps, closures live here.
    pub heap: Heap,
    /// FFI registrations.
    pub ffi: CallTable,
}

impl Vm {
    /// Constructs a fresh VM around `module`. The GC defaults to a
    /// 250 µs per-tick budget (see [`crate::gc::GcConfig`]).
    pub fn new(module: Module) -> Self {
        Self {
            module,
            heap: Heap::with_default_config(),
            ffi: CallTable::new(),
        }
    }

    /// Invokes the named function with `args`. Returns the function's
    /// return value, or a stop reason explaining why execution halted
    /// before normal return.
    pub fn call(&mut self, name: &str, args: Vec<Value>) -> StopReason {
        let Some(fn_id) = self.module.function_id(name) else {
            return StopReason::Error(format!("no function named `{name}`"));
        };
        let max_register = self.module.functions[fn_id as usize].max_register;
        let mut frame = CallFrame::new(fn_id, max_register, None);
        for (i, v) in args.into_iter().enumerate() {
            frame.set_reg(i as u8, v);
        }
        let mut stack = vec![frame];
        dispatch::run(&self.module, &mut stack, &mut self.heap, &self.ffi)
    }
}
