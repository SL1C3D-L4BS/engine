//! Runtime value tag for the sli VM.
//!
//! `Value` is the union the dispatch loop and the FFI marshal layer both
//! see. Primitive variants are unboxed; aggregates (strings, arrays,
//! maps, structs, closures) ride behind a [`GcHandle`] so the GC owns
//! their storage and lifetime (see [`crate::gc`]).

use crate::gc::GcHandle;
use std::sync::Arc;

/// A runtime value.
#[derive(Clone, Debug, Default)]
pub enum Value {
    /// `nil`
    #[default]
    Nil,
    /// Boolean.
    Bool(bool),
    /// Signed 64-bit integer.
    Int(i64),
    /// 64-bit float.
    Float(f64),
    /// UTF-8 string. Interned via `Arc` so cloning is a refcount bump.
    Str(Arc<str>),
    /// Heap-allocated array of values.
    Array(GcHandle),
    /// Heap-allocated map (string key → value).
    Map(GcHandle),
    /// Heap-allocated struct instance.
    Struct(GcHandle),
    /// Closure capturing 0+ upvalues.
    Closure(GcHandle),
    /// ECS entity handle — opaque 64-bit id.
    Entity(u64),
    /// Type-erased asset / FFI handle.
    Handle(u64),
}

impl Value {
    /// `true` for `Value::Nil`.
    pub fn is_nil(&self) -> bool {
        matches!(self, Self::Nil)
    }

    /// `true` for `Value::Bool(true)` and only that.
    pub fn truthy(&self) -> bool {
        matches!(self, Self::Bool(true))
    }

    /// Returns the inner integer or panics on tag mismatch — used by
    /// the dispatch loop on opcodes whose verifier already proved the
    /// tag (`AddInt` etc).
    pub fn as_int(&self) -> i64 {
        match self {
            Self::Int(v) => *v,
            _ => panic!("expected int, found {self:?}"),
        }
    }

    /// Returns the inner float or panics on tag mismatch.
    pub fn as_float(&self) -> f64 {
        match self {
            Self::Float(v) => *v,
            _ => panic!("expected float, found {self:?}"),
        }
    }

    /// Returns the inner bool or panics on tag mismatch.
    pub fn as_bool(&self) -> bool {
        match self {
            Self::Bool(v) => *v,
            _ => panic!("expected bool, found {self:?}"),
        }
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Nil, Self::Nil) => true,
            (Self::Bool(a), Self::Bool(b)) => a == b,
            (Self::Int(a), Self::Int(b)) => a == b,
            (Self::Float(a), Self::Float(b)) => a.to_bits() == b.to_bits(),
            (Self::Str(a), Self::Str(b)) => a == b,
            (Self::Entity(a), Self::Entity(b)) => a == b,
            (Self::Handle(a), Self::Handle(b)) => a == b,
            // GC-backed values compare by handle identity. Structural
            // equality lives behind a script-level `==` builtin we have
            // not surfaced yet.
            (Self::Array(a), Self::Array(b)) => a == b,
            (Self::Map(a), Self::Map(b)) => a == b,
            (Self::Struct(a), Self::Struct(b)) => a == b,
            (Self::Closure(a), Self::Closure(b)) => a == b,
            _ => false,
        }
    }
}

/// A short debug summary suitable for the debugger LOCALS pane. Avoids
/// recursion into GC-backed aggregates.
pub fn summary(v: &Value) -> String {
    match v {
        Value::Nil => "nil".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => format!("{f}"),
        Value::Str(s) => format!("{:?}", s.as_ref()),
        Value::Array(h) => format!("Array({h:?})"),
        Value::Map(h) => format!("Map({h:?})"),
        Value::Struct(h) => format!("Struct({h:?})"),
        Value::Closure(h) => format!("Closure({h:?})"),
        Value::Entity(e) => format!("Entity({e})"),
        Value::Handle(h) => format!("Handle({h:#x})"),
    }
}
