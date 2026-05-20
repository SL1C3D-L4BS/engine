//! [`ReflectValue`] — the dynamic value type used by reflection.
//!
//! Reflection needs a single type that can carry the value of *any* primitive
//! field across the dynamic boundary (editor inspector, serialization, script
//! bindings). `ReflectValue` is that type. Conversions in both directions are
//! provided for every primitive the engine treats as a leaf value.

/// A dynamically typed primitive value produced or consumed by reflection.
#[derive(Clone, Debug, PartialEq)]
pub enum ReflectValue {
    /// A boolean.
    Bool(bool),
    /// A signed 32-bit integer.
    I32(i32),
    /// A signed 64-bit integer.
    I64(i64),
    /// An unsigned 32-bit integer.
    U32(u32),
    /// An unsigned 64-bit integer.
    U64(u64),
    /// A 32-bit float.
    F32(f32),
    /// A 64-bit float.
    F64(f64),
    /// An owned string.
    String(String),
}

macro_rules! into_reflect {
    ($t:ty, $variant:ident) => {
        impl From<$t> for ReflectValue {
            #[inline]
            fn from(v: $t) -> Self {
                ReflectValue::$variant(v)
            }
        }
    };
}

into_reflect!(bool, Bool);
into_reflect!(i32, I32);
into_reflect!(i64, I64);
into_reflect!(u32, U32);
into_reflect!(u64, U64);
into_reflect!(f32, F32);
into_reflect!(f64, F64);
into_reflect!(String, String);

/// Fallible conversion *out* of a [`ReflectValue`] back to a concrete type.
///
/// This is the trait the `Reflect` derive calls in its generated `set_field`;
/// it returns `None` when the dynamic value's type does not match.
pub trait FromReflect: Sized {
    /// Attempts to recover a `Self` from a dynamic value.
    fn from_reflect(value: ReflectValue) -> Option<Self>;
}

macro_rules! from_reflect {
    ($t:ty, $variant:ident) => {
        impl FromReflect for $t {
            #[inline]
            fn from_reflect(value: ReflectValue) -> Option<Self> {
                match value {
                    ReflectValue::$variant(v) => Some(v),
                    _ => None,
                }
            }
        }
    };
}

from_reflect!(bool, Bool);
from_reflect!(i32, I32);
from_reflect!(i64, I64);
from_reflect!(u32, U32);
from_reflect!(u64, U64);
from_reflect!(f32, F32);
from_reflect!(f64, F64);
from_reflect!(String, String);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_every_primitive() {
        assert_eq!(f32::from_reflect(ReflectValue::from(2.5f32)), Some(2.5));
        assert_eq!(i64::from_reflect(ReflectValue::from(-9i64)), Some(-9));
        assert_eq!(bool::from_reflect(ReflectValue::from(true)), Some(true));
        assert_eq!(
            String::from_reflect(ReflectValue::from(String::from("hi"))),
            Some(String::from("hi"))
        );
    }

    #[test]
    fn type_mismatch_yields_none() {
        assert_eq!(i32::from_reflect(ReflectValue::F32(1.0)), None);
        assert_eq!(f64::from_reflect(ReflectValue::Bool(false)), None);
    }
}
