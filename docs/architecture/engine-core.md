# engine-core

The ECS, system scheduler, deterministic RNG, arena allocators, and telemetry
primitives (spec IV.1 Level 1, IV.2, IV.3, X.1–X.3, XVI).

## Purpose

The largest foundation crate and the one carrying the most contract: it is
where entities live, where systems are ordered, where random numbers come
from, and where telemetry signals are first recorded.

## Modules

| Module          | Contents |
|-----------------|----------|
| `ecs::entity`   | `Entity` = `u64` `[generation:u32 \| index:u32]`; index recycling and generation invalidation. |
| `ecs::storage`  | `DenseColumn` (Table storage) and `SparseColumn` (SparseSet storage) — the hybrid model of ADR-002. |
| `ecs::world`    | `World` — spawn/despawn, add/remove components, typed iteration, `Res` resources. |
| `ecs::schedule` | `Schedule` — systems grouped into the fixed `Phase`s, run in a stable, deterministic order (spec IV.2). |
| `rng`           | `rand(frame, channel, counter)` = BLAKE3 over `(seed ‖ frame ‖ channel ‖ counter)` — stateless, no global RNG. |
| `alloc`         | `LinearArena`, `RingArena`, `PoolArena` — explicit-lifetime allocators (spec XVI). |
| `telemetry`     | The owned `Signal` types (span/counter/gauge/event) and the per-thread loss-tolerant ring buffer; `span!` / `counter_inc!` / `gauge_set!` / `event!` macros. |

## Determinism

- Iteration over component storage is always in ascending entity-index order,
  regardless of storage backend.
- The scheduler's order is a stable topological sort keyed by
  `(phase, registration index)` — identical across runs and across single- vs
  multi-threaded execution.
- The RNG has no global state; identical `(seed, frame, channel, counter)`
  always yields identical output, on every architecture.

## Oracles

- `tests/ecs.rs` — entity lifecycle, both storage backends, iteration order,
  the scheduler, and resources.
- `tests/determinism.rs` + `tests/golden-rng.txt` — the RNG sequence and a
  scripted ECS build reduced to an FNV-1a digest, asserted against a committed
  golden; run cross-arch in CI (ADR-013, ADR-023).

## Dependencies

`engine-math`, `engine-reflect`, `engine-ecs-macro`, `blake3` — Level 1.
