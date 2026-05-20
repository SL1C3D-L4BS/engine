# engine-ecs-macro

The workspace's procedural derive macros (spec IV.1 Level 0, ADR-024).

## Purpose

Rust requires derive macros to live in a dedicated `proc-macro` crate. Per
ADR-024 this crate hosts *all* of the engine's derive macros — not only ECS
ones — because the spec's Level 0 crate list names exactly one proc-macro
crate.

## Macros

| Macro                 | Generates |
|-----------------------|-----------|
| `#[derive(Component)]` | An `engine_core::ecs::Component` impl. The `#[component(storage = "Table" \| "SparseSet")]` attribute selects the storage backend; default is `Table`. |
| `#[derive(Reflect)]`   | An `engine_reflect::Reflect` impl for a named-field struct — field count, names, and dynamic `get_field` / `set_field`. |

## Design notes

- Generated code uses fully-qualified absolute paths
  (`::engine_core::…`, `::engine_reflect::…`), so it is hygienic and does not
  depend on what the consumer has imported.
- An unknown `storage` value or a non-named-field struct produces a
  `compile_error!` with a clear message, not a confusing downstream type error.

## Oracle

The macros' behaviour is verified by their *consumers*: `engine-core`'s ECS
tests exercise `#[derive(Component)]` for both storage kinds, and
`engine-reflect`'s `tests/derive.rs` round-trips every primitive field type
through a `#[derive(Reflect)]` struct.

## Dependencies

`syn`, `quote`, `proc-macro2` — Level 0.
