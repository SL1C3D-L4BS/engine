//! `engine-shader` — Slang shader toolchain (Phase 4 PR 4).
//!
//! Wraps the official `slangc` binary as a sandboxed subprocess
//! (ADR-019) and exposes a per-target compile API for the Phase-5
//! renderer. Four canonical targets ride through the pipeline:
//! SPIR-V (Vulkan), WGSL (WebGPU), DXIL (DirectX 12), MSL (Metal).
//!
//! # Why a subprocess
//!
//! Spec ADR-003 names Slang as the source language, *not* as
//! something we re-implement. Sliced Engine owns the compilation
//! frame, the artefact format, the asset-pipeline integration, and
//! the reproducibility golden — but the language semantics live in
//! `slangc`. A subprocess wrapper is the right abstraction: one
//! binary, pinned version (see [`slangc::SLANGC_PIN`]), explicit
//! flags, captured stderr.
//!
//! # Reproducibility
//!
//! Each [`Artifact`] carries a BLAKE3 digest over its compiled
//! bytes. The cross-arch oracle in
//! `tests/reproducibility.rs` re-compiles a fixed corpus, takes a
//! per-target digest, and compares against the committed golden.
//! A pinned `slangc` plus byte-equal output across x86-64 and
//! aarch64 makes the asset pak content-addressable
//! (ADR-008 + ADR-038).
//!
//! # Module map
//!
//! - [`target`] — `Target` (SPIR-V / WGSL / DXIL / MSL) + `Stage`.
//! - [`slangc`] — sandboxed subprocess wrapper + version pin.
//! - [`artifact`] — `Artifact`, `Bundle`, on-disk encoding, `impl Asset`.

pub mod artifact;
pub mod slangc;
pub mod target;

pub use artifact::{Artifact, BUNDLE_MAGIC, BUNDLE_VERSION, Bundle, DecodeError, decode, encode};
pub use slangc::{Compiler, SLANGC_PIN, SlangcError};
pub use target::{Stage, Target};

use std::path::Path;

/// Compiles `source` for every target in [`Target::all`]. Skips a
/// target if `slangc` reports a hard failure for it but continues
/// with the rest, accumulating the per-target errors. Returns the
/// bundle plus any per-target compile errors.
///
/// On a clean run, the error list is empty and the bundle has four
/// artefacts. The Phase-5 renderer can drop the unused targets per
/// platform.
pub fn compile_all_targets(
    compiler: &Compiler,
    source: &Path,
    entry: &str,
    stage: Stage,
) -> (Bundle, Vec<SlangcError>) {
    let mut artefacts = Vec::with_capacity(Target::all().len());
    let mut errors = Vec::new();
    for &target in Target::all() {
        match compiler.compile_with_reflection(source, entry, stage, target, None) {
            Ok((bytes, refl)) => {
                artefacts.push(Artifact::new(target, bytes, refl.unwrap_or_default()));
            }
            Err(e) => errors.push(e),
        }
    }
    (Bundle::new(entry, stage, artefacts), errors)
}
