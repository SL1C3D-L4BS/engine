//! The [`Reflect`] trait — runtime field introspection.

use crate::value::ReflectValue;

/// Runtime reflection over a type's named fields.
///
/// Implemented by `#[derive(Reflect)]` (from `engine-ecs-macro`). The trait is
/// object-safe, so reflected values can be handled as `&dyn Reflect` — this is
/// what the editor's INSPECTOR form generator and the script bindings consume.
pub trait Reflect {
    /// The type's name, as written in source.
    fn type_name(&self) -> &'static str;

    /// The number of reflected fields.
    fn field_count(&self) -> usize;

    /// The name of the field at `index`, or `None` if out of range.
    fn field_name(&self, index: usize) -> Option<&'static str>;

    /// Reads the named field as a dynamic value, or `None` if no such field.
    fn get_field(&self, name: &str) -> Option<ReflectValue>;

    /// Writes the named field from a dynamic value.
    ///
    /// Returns `false` if the field does not exist or the value's type does
    /// not match the field's type; in that case the field is left unchanged.
    fn set_field(&mut self, name: &str, value: ReflectValue) -> bool;

    /// Collects every field name in declaration order.
    fn field_names(&self) -> Vec<&'static str> {
        (0..self.field_count())
            .filter_map(|i| self.field_name(i))
            .collect()
    }
}
