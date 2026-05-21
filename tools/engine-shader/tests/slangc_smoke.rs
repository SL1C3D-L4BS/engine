//! `slangc` subprocess smoke oracle (PR 4, ADR-037).
//!
//! Skips with an informational message if `slangc` is not installed
//! or if the installed version does not match `SLANGC_PIN`. When it
//! runs, asserts each target compiles for the fixture entry point;
//! DXIL requires a `dxc` library that ships only on Windows, so a
//! `dxil` failure is recorded but not fatal.

use engine_shader::slangc::{Compiler, SLANGC_PIN, SlangcError};
use engine_shader::target::{Stage, Target};
use std::path::PathBuf;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/triangle.slang")
}

fn locate_or_skip() -> Option<Compiler> {
    match Compiler::permissive() {
        Ok(c) => Some(c),
        Err(SlangcError::NotFound) => {
            eprintln!("slangc not installed — skipping ({}).", file!());
            None
        }
        Err(e) => panic!("locating slangc failed: {e}"),
    }
}

#[test]
fn version_pin_advisory() {
    let Some(c) = locate_or_skip() else {
        return;
    };
    if c.version() != SLANGC_PIN {
        eprintln!(
            "warning: slangc version {} does not match SLANGC_PIN={SLANGC_PIN}; \
             reproducibility goldens will not match. (ADR-038)",
            c.version()
        );
    }
}

#[test]
fn spirv_vertex_compiles() {
    let Some(c) = locate_or_skip() else {
        return;
    };
    let bytes = c
        .compile(&fixture(), "vs_main", Stage::Vertex, Target::SpirV)
        .expect("SPIR-V vertex");
    // SPIR-V binaries begin with the magic 0x07230203.
    assert!(bytes.len() > 4, "SPIR-V too short: {} bytes", bytes.len());
    assert_eq!(
        &bytes[..4],
        &[0x03, 0x02, 0x23, 0x07],
        "SPIR-V magic missing"
    );
}

#[test]
fn wgsl_fragment_compiles() {
    let Some(c) = locate_or_skip() else {
        return;
    };
    let bytes = c
        .compile(&fixture(), "fs_main", Stage::Fragment, Target::Wgsl)
        .expect("WGSL fragment");
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        text.contains("fragment") || text.contains("@fragment"),
        "WGSL output missing @fragment qualifier: {text}"
    );
}

#[test]
fn msl_compute_compiles() {
    let Some(c) = locate_or_skip() else {
        return;
    };
    let bytes = c
        .compile(&fixture(), "cs_main", Stage::Compute, Target::Msl)
        .expect("MSL compute");
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        text.contains("kernel") || text.contains("[[kernel]]") || text.contains("metal_stdlib"),
        "MSL output missing kernel qualifier or metal stdlib include"
    );
}

#[test]
fn dxil_requires_dxc() {
    let Some(c) = locate_or_skip() else {
        return;
    };
    // DXIL needs the Microsoft `dxcompiler` shared library. On
    // Linux CI runners it is normally absent; record the result
    // either way without failing the build.
    match c.compile(&fixture(), "vs_main", Stage::Vertex, Target::Dxil) {
        Ok(bytes) => {
            eprintln!("DXIL compiled: {} bytes", bytes.len());
            assert!(!bytes.is_empty());
        }
        Err(SlangcError::Compile { stderr, .. }) => {
            eprintln!("DXIL not available on this host (expected on non-Windows):\n{stderr}");
        }
        Err(e) => panic!("unexpected DXIL error: {e}"),
    }
}

#[test]
fn empty_path_returns_compile_error() {
    let Some(c) = locate_or_skip() else {
        return;
    };
    let nonexistent = PathBuf::from("/dev/null/does_not_exist.slang");
    let err = c
        .compile(&nonexistent, "vs_main", Stage::Vertex, Target::SpirV)
        .expect_err("expected error");
    assert!(matches!(err, SlangcError::Compile { .. }));
}
