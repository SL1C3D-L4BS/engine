# engine-reflect

Runtime reflection and type registration (spec IV.1 Level 0, III.2, IV.7).

## Purpose

Lets tooling inspect and mutate values whose concrete type is only known at
runtime. It backs the editor's reflection-driven INSPECTOR form generator and
the auto-registered ECS script bindings.

## Modules

| Module     | Contents |
|------------|----------|
| `reflect`  | The `Reflect` trait — type name, field count, field names, dynamic `get_field` / `set_field`. Object-safe, so values flow as `&dyn Reflect`. |
| `value`    | `ReflectValue` — the dynamic value enum carried across the reflection boundary; `FromReflect` converts back to a concrete type. |
| `registry` | `TypeRegistry` — a name-keyed table of `TypeInfo` structural metadata, for scene files and script bindings that refer to types by name. |

## Design notes

- `Reflect` is derived by `#[derive(Reflect)]` from `engine-ecs-macro`
  (ADR-024); hand-written impls are possible but rare.
- `ReflectValue` carries every primitive the engine treats as a leaf value;
  conversions exist in both directions.
- The registry is ordered (`BTreeMap`), so enumeration is deterministic.

## Oracle

`tests/derive.rs` is the derive round-trip oracle: for a struct with every
primitive field type, each field is read via `get_field` and written via
`set_field`, asserting the value survives the dynamic round-trip.

## Dependencies

`std` only — Level 0. (`engine-ecs-macro` is a dev-dependency, used by tests
to exercise the derive.)
