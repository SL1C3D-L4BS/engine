//! Oracle for the `#[derive(Reflect)]` macro: a derived implementation must
//! round-trip every field through the dynamic [`ReflectValue`] boundary.

use engine_ecs_macro::Reflect;
use engine_reflect::{Reflect as _, ReflectValue, TypeRegistry};

#[derive(Reflect, Default)]
struct Transform {
    x: f32,
    y: f32,
    z: f32,
    frozen: bool,
    id: u64,
}

#[test]
fn derived_reflect_round_trips_every_field() {
    let mut t = Transform::default();
    assert_eq!(t.type_name(), "Transform");
    assert_eq!(t.field_count(), 5);
    assert_eq!(t.field_names(), vec!["x", "y", "z", "frozen", "id"]);

    assert!(t.set_field("x", ReflectValue::F32(1.5)));
    assert!(t.set_field("frozen", ReflectValue::Bool(true)));
    assert!(t.set_field("id", ReflectValue::U64(42)));

    assert_eq!(t.get_field("x"), Some(ReflectValue::F32(1.5)));
    assert_eq!(t.get_field("frozen"), Some(ReflectValue::Bool(true)));
    assert_eq!(t.get_field("id"), Some(ReflectValue::U64(42)));
}

#[test]
fn type_mismatched_writes_are_rejected() {
    let mut t = Transform::default();
    assert!(t.set_field("x", ReflectValue::F32(9.0)));
    // A bool cannot be stored into an f32 field; the field is left untouched.
    assert!(!t.set_field("x", ReflectValue::Bool(true)));
    assert_eq!(t.get_field("x"), Some(ReflectValue::F32(9.0)));
}

#[test]
fn unknown_fields_are_absent() {
    let mut t = Transform::default();
    assert!(t.get_field("missing").is_none());
    assert!(!t.set_field("missing", ReflectValue::F32(0.0)));
    assert!(t.field_name(99).is_none());
}

#[test]
fn registry_records_a_derived_type() {
    let mut registry = TypeRegistry::new();
    assert!(registry.is_empty());
    registry.register(&Transform::default());

    assert!(registry.contains("Transform"));
    assert_eq!(registry.len(), 1);
    let info = registry.get("Transform").expect("registered");
    assert_eq!(info.field_names, vec!["x", "y", "z", "frozen", "id"]);
}
