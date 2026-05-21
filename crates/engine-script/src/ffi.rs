//! FFI call table — sli ↔ Rust bridge.
//!
//! A Rust function registered here becomes callable from sli bytecode
//! via the `FfiCall` opcode. The call table is indexed by `u16` so the
//! lowered bytecode resolves names at compile time, not on every call.
//!
//! The marshal layer is intentionally minimal in PR 2: callbacks receive
//! and return raw [`Value`]s, and any tag mismatch surfaces as the
//! function's own error string. PR 3 will widen this with
//! engine-reflect-driven `ScriptArg`/`ScriptRet` traits so ECS bindings
//! (`Query<T>`, `Res<T>`, `ResMut<T>`) auto-register.

use crate::gc::Heap;
use crate::vm::Value;

/// Function signature exposed to sli code.
pub type FfiFn = fn(args: &[Value], heap: &mut Heap) -> Result<Value, String>;

/// A registered FFI binding.
#[derive(Clone, Debug)]
pub struct Binding {
    /// Name as seen from sli source.
    pub name: String,
    /// Expected argument count. `None` means variadic.
    pub arity: Option<u8>,
    /// Implementation.
    pub callback: FfiFn,
}

/// Owned registry of FFI bindings.
#[derive(Debug, Default)]
pub struct CallTable {
    bindings: Vec<Binding>,
}

impl CallTable {
    /// An empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a binding, returning its assigned index. Indexes are
    /// stable across runs of the same registration order — PR 3's
    /// hot-reload re-runs registration before code swap.
    pub fn register(&mut self, name: impl Into<String>, arity: Option<u8>, callback: FfiFn) -> u16 {
        let id = self.bindings.len() as u16;
        self.bindings.push(Binding {
            name: name.into(),
            arity,
            callback,
        });
        id
    }

    /// Looks up a binding by name.
    pub fn id_of(&self, name: &str) -> Option<u16> {
        self.bindings
            .iter()
            .position(|b| b.name == name)
            .map(|i| i as u16)
    }

    /// Borrows a binding by index.
    pub fn binding(&self, id: u16) -> Option<&Binding> {
        self.bindings.get(id as usize)
    }

    /// Dispatches a call.
    pub fn call(&self, id: u16, args: &[Value], heap: &mut Heap) -> Result<Value, String> {
        let Some(b) = self.bindings.get(id as usize) else {
            return Err(format!("no ffi binding at index {id}"));
        };
        if let Some(expected) = b.arity
            && expected as usize != args.len()
        {
            return Err(format!(
                "ffi `{}` expected {expected} args, got {}",
                b.name,
                args.len()
            ));
        }
        (b.callback)(args, heap)
    }

    /// Number of registered bindings.
    pub fn len(&self) -> usize {
        self.bindings.len()
    }

    /// Whether the table is empty.
    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }
}
