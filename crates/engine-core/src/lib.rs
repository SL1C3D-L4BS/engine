//! `engine-core` — the ECS, system scheduler, deterministic RNG, arena
//! allocators, and telemetry primitives.
//!
//! Level 1 crate. See `ENGINE_SPECIFICATION_v2.0.md` Part IV.1.
//!
//! # Modules
//!
//! - [`ecs`] — entities, components, the [`World`], and the [`Schedule`].
//! - [`rng`] — BLAKE3-keyed deterministic random numbers (spec IV.2).
//! - [`alloc`] — linear, ring, pool, and general-purpose arena allocators
//!   under the uniform [`alloc::Arena`] accounting trait.
//! - [`collections`] — owned Robin Hood hash map and its two hashers
//!   (ADR-028). Replaces [`std::collections::HashMap`] across the engine
//!   crates so the probe path is owned and tail-latency is bounded.
//! - [`telemetry`] — the owned span / counter / gauge / event primitives and
//!   the per-thread ring buffer (spec X.1–X.3).
//!
//! # Determinism
//!
//! The ECS scheduler order and the RNG are deterministic by construction
//! (ADR-013); `tests/determinism.rs` is the cross-architecture oracle that
//! pins this with a committed golden digest.

pub mod alloc;
pub mod collections;
pub mod ecs;
pub mod rng;
pub mod telemetry;

pub use ecs::{Entity, Phase, Schedule, StorageKind, World};
pub use rng::Rng;

// The `Component` trait (from `ecs`) and the `#[derive(Component)]` macro
// (from `engine-ecs-macro`, per ADR-024) are both re-exported here under the
// name `Component`; they occupy different namespaces, so a single
// `use engine_core::Component;` brings in both.
pub use ecs::Component;
pub use engine_ecs_macro::Component;
