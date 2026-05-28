//! Push-constant + bind-group ABI contracts shared between Rust and
//! WGSL (ADR-077 §8).
//!
//! Mirrors the ADR-064 / ADR-065 contract-module pattern: every
//! piece of cross-language data structure has a single canonical
//! definition here, and the shaders + the Rust pass implementation
//! both reference it.

/// Splat-sort compute shader: per-pass workgroup size (1D).
/// Matches `@workgroup_size(SPLAT_SORT_WORKGROUP_SIZE, 1, 1)` in
/// `crates/engine-render/shaders/splat_sort.wgsl`.
pub const SPLAT_SORT_WORKGROUP_SIZE: u32 = 256;

/// Splat-sort radix-pass count (4 passes × 8 bits = full 32-bit key).
pub const SPLAT_SORT_RADIX_PASSES: u32 = 4;

/// Splat-sort radix-pass bin count (2^8).
pub const SPLAT_SORT_BINS: u32 = 256;

/// Splat-composite WGSL push-constant size in bytes (ADR-077 §8).
/// Matches the `composite::PushConstants` struct in this crate.
pub const SPLAT_COMPOSITE_PUSH_CONSTANTS_BYTES: usize = 80;

/// Bind-group layout id for the splat-composite pass's storage
/// buffers (positions, scales, rotations, colors, opacities,
/// sorted_perm).
pub const SPLAT_COMPOSITE_BG_STORAGE: u32 = 0;

/// Bind-group layout id for the splat-composite pass's textures
/// (gbuffer_depth read; scene_color read-write).
pub const SPLAT_COMPOSITE_BG_TEXTURES: u32 = 1;
